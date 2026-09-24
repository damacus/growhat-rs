use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use growhat::{
    config::{Backend, Config},
    hardware::HardwareSource,
    pump::{self, GpioPumpDriver, PumpCommand, PumpOutcome},
    sensor::{ReadingStatus, SensorSource, SensorTracker, SimulatedSource},
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[path = "mqtt.rs"]
mod mqtt;

#[derive(Parser)]
#[command(
    version,
    about = "Grow HAT Mini moisture readings and Home Assistant MQTT discovery",
    after_help = "Examples:\n  growhat --config examples/simulated.toml diagnose --samples 3\n  growhat --config config.local.toml check-config\n  growhat --config config.local.toml run\n\nDiagnostics print one JSON object per sensor per sample to stdout.\nLogs and errors use stderr. Exit codes: 0 success, 1 runtime/reading failure, 2 usage/configuration error."
)]
struct Cli {
    #[arg(long, global = true, default_value = "config.local.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate configuration without opening GPIO or connecting to MQTT
    CheckConfig,
    /// Read sensors a bounded number of times without MQTT; output NDJSON
    Diagnose {
        #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=100))]
        samples: u32,
    },
    /// Run one bounded, explicitly configured pump pulse without MQTT
    Pump {
        #[arg(long)]
        request_id: String,
        #[arg(long)]
        channel: u8,
        #[arg(long)]
        duty_percent: u8,
        #[arg(long)]
        duration_ms: u64,
    },
    /// Publish readings and discovery until SIGINT or SIGTERM
    Run,
}

fn source(config: &Config) -> Result<Box<dyn SensorSource>> {
    match config.backend {
        Backend::Simulated => Ok(Box::new(SimulatedSource::new(config))),
        Backend::Hardware => Ok(Box::new(HardwareSource::new(config).context(
            "opening moisture inputs; verify board, GPIO chip and line ownership",
        )?)),
    }
}

fn diagnose(config: &Config, samples: u32) -> Result<()> {
    let mut source = source(config)?;
    let mut tracker = SensorTracker::new(config);
    let start = Instant::now();
    let mut healthy = true;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for index in 0..samples {
        let cycle_start = Instant::now();
        let readings = tracker.update(source.read(), start.elapsed());
        for reading in readings {
            healthy &= matches!(
                reading.status,
                ReadingStatus::Valid | ReadingStatus::Uncalibrated
            );
            serde_json::to_writer(&mut output, &reading)?;
            writeln!(&mut output)?;
        }
        output.flush()?;
        if index + 1 < samples {
            std::thread::sleep(
                Duration::from_millis(config.sample_interval_ms)
                    .saturating_sub(cycle_start.elapsed()),
            );
        }
    }
    if !healthy {
        bail!(
            "SENSOR_INVALID: diagnostic contained invalid or stale samples; inspect the JSON error fields and sensor connections"
        );
    }
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn serve(config: Config) -> Result<()> {
    if config.mqtt.is_none() {
        bail!("MQTT_CONFIG_MISSING: run requires [mqtt]; use diagnose for sensor-only checks");
    }
    let mut source = source(&config)?;
    let initial = mqtt::Batch {
        readings: SensorTracker::new(&config).update(Vec::new(), Duration::ZERO),
        captured_at: Instant::now(),
    };
    let (sender, receiver) = watch::channel(initial);
    let worker_stop = Arc::new(AtomicBool::new(false));
    let mut panel = if config.panel_enabled {
        Some(growhat::panel::start(
            &config.gpio_chip,
            Arc::clone(&worker_stop),
        )?)
    } else {
        None
    };
    let panel_commands = panel.as_ref().map(|panel| panel.commands.clone());
    let (unused_sender, empty_events) = tokio::sync::mpsc::channel(1);
    drop(unused_sender);
    let button_events = panel
        .as_mut()
        .map(|panel| std::mem::replace(&mut panel.events, tokio::sync::mpsc::channel(1).1))
        .unwrap_or(empty_events);
    let stopped = Arc::clone(&worker_stop);
    let worker_config = config.clone();
    let sensor_panel_commands = panel_commands.clone();
    // GPIO waits must not stall keepalives, broker reconnects or signal handling.
    let worker = std::thread::spawn(move || {
        let mut tracker = SensorTracker::new(&worker_config);
        let start = Instant::now();
        while !stopped.load(Ordering::Relaxed) {
            let cycle_start = Instant::now();
            let readings = tracker.update(source.read(), start.elapsed());
            if let Some(commands) = &sensor_panel_commands {
                let _ = commands.try_send(growhat::panel::PanelCommand::Readings(readings.clone()));
            }
            if sender
                .send(mqtt::Batch {
                    readings,
                    captured_at: Instant::now(),
                })
                .is_err()
            {
                break;
            }
            let deadline = cycle_start + Duration::from_millis(worker_config.sample_interval_ms);
            while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                std::thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
            }
        }
    });
    let (stop_sender, stop_receiver) = watch::channel(false);
    let signal_task = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop_sender.send(true);
    });
    let result = tokio::select! {
        result = mqtt::serve(&config, receiver, stop_receiver, panel_commands, button_events) => result,
        _ = async {
            loop {
                if panel.as_ref().is_some_and(|panel| panel.worker.is_finished()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }, if panel.is_some() => Err(anyhow::anyhow!("panel worker stopped unexpectedly")),
    };
    worker_stop.store(true, Ordering::Relaxed);
    worker.thread().unpark();
    signal_task.abort();
    // An in-flight read is input-only and bounded. Do not make shutdown wait for
    // a potentially long user-configured measurement window.
    drop(worker);
    if let Some(panel) = panel {
        // The panel loop uses short bounded waits and turns the backlight off.
        panel
            .worker
            .join()
            .map_err(|_| anyhow::anyhow!("panel worker panicked"))??;
    }
    result
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();
    let config = match Config::load(&cli.config) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("CONFIG_INVALID: {error:#}. Check --config and the example TOML files.");
            std::process::exit(2);
        }
    };
    let result = match cli.command {
        Command::CheckConfig => {
            println!("Configuration valid");
            Ok(())
        }
        Command::Diagnose { samples } => diagnose(&config, samples),
        Command::Pump {
            request_id,
            channel,
            duty_percent,
            duration_ms,
        } => {
            let command = PumpCommand {
                request_id,
                channel,
                duty_percent,
                duration_ms,
            };
            let driver = GpioPumpDriver {
                chip: config.gpio_chip.clone(),
            };
            match pump::execute_local(&config, &command, &driver) {
                Ok(PumpOutcome::Completed) => {
                    println!("Pump pulse completed: {}", command.request_id);
                    Ok(())
                }
                Ok(PumpOutcome::Duplicate) => {
                    println!("Pump request already recorded: {}", command.request_id);
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        Command::Run => serve(config).await,
    };
    if let Err(error) = result {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
