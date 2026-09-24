//! Bounded, manual Grow HAT pump pulses. No sensor reading starts a pump.
use crate::config::{Config, PumpConfig};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DAY_MS: u64 = 86_400_000;
// A typo in a manual debug command must still have a finite hardware cutoff.
const MAX_DEBUG_PULSE_MS: u64 = 300_000;
#[cfg(target_os = "linux")]
const PWM_PERIOD: Duration = Duration::from_millis(10);
#[cfg(target_os = "linux")]
const PINS: [u32; 3] = [17, 27, 22];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PumpCommand {
    pub request_id: String,
    pub channel: u8,
    pub duty_percent: u8,
    pub duration_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PumpOutcome {
    Completed,
    Duplicate,
}

#[derive(Default, Deserialize, Serialize)]
struct Ledger {
    entries: Vec<LedgerEntry>,
}

#[derive(Deserialize, Serialize)]
struct LedgerEntry {
    request_id: String,
    at_ms: u64,
    duration_ms: u64,
}

pub fn validate<'a>(config: &'a Config, command: &PumpCommand) -> Result<&'a PumpConfig> {
    validate_with_duration(config, command, false)
}

fn validate_with_duration<'a>(
    config: &'a Config,
    command: &PumpCommand,
    local_debug: bool,
) -> Result<&'a PumpConfig> {
    let pump = config.pump.as_ref().context("pump not configured")?;
    if !pump.enabled {
        bail!("pump disabled");
    }
    if !config.hardware_confirmed || !matches!(config.backend, crate::config::Backend::Hardware) {
        bail!("pump requires confirmed hardware");
    }
    if !pump.channels.contains(&command.channel) {
        bail!("pump channel not enabled");
    }
    if !valid_id(&command.request_id) {
        bail!("request_id must be 1..64 ASCII letters, digits, underscores or hyphens");
    }
    if !(1..=90).contains(&command.duty_percent) {
        bail!("duty_percent must be between 1 and 90");
    }
    let max_duration_ms = if local_debug {
        MAX_DEBUG_PULSE_MS
    } else {
        pump.max_duration_ms
    };
    if !(1..=max_duration_ms).contains(&command.duration_ms) {
        bail!("duration_ms exceeds configured maximum");
    }
    Ok(pump)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn now_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes Unix epoch")?
        .as_millis()
        .try_into()
        .context("system clock out of range")
}

struct PulseLock {
    _file: File,
}

impl PulseLock {
    fn acquire(state_file: &Path) -> Result<Self> {
        let lock_path = state_file.with_extension("lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
            .with_context(|| format!("open pump lock {}", lock_path.display()))?;
        #[cfg(target_os = "linux")]
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!("another pump pulse is running");
        }
        Ok(Self { _file: file })
    }
}

fn reserve(
    pump: &PumpConfig,
    command: &PumpCommand,
    at_ms: u64,
    bypass_daily_budget: bool,
) -> Result<PumpOutcome> {
    let mut ledger: Ledger = match fs::read(&pump.state_file) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).context("pump state invalid; refusing pulse")?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
        Err(error) => return Err(error).context("read pump state"),
    };
    ledger
        .entries
        .retain(|entry| entry.at_ms.saturating_add(DAY_MS) > at_ms);
    if ledger
        .entries
        .iter()
        .any(|entry| entry.request_id == command.request_id)
    {
        return Ok(PumpOutcome::Duplicate);
    }
    let used = ledger
        .entries
        .iter()
        .fold(0u64, |sum, entry| sum.saturating_add(entry.duration_ms));
    if !bypass_daily_budget && used.saturating_add(command.duration_ms) > pump.max_total_ms_per_24h
    {
        bail!("pump 24-hour duration limit reached");
    }
    ledger.entries.push(LedgerEntry {
        request_id: command.request_id.clone(),
        at_ms,
        duration_ms: command.duration_ms,
    });
    write_ledger(&pump.state_file, &ledger)?;
    Ok(PumpOutcome::Completed)
}

fn write_ledger(path: &Path, ledger: &Ledger) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .with_context(|| format!("create pump state temporary file {}", temp.display()))?;
    let result = (|| -> Result<()> {
        serde_json::to_writer(&mut file, ledger)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.context("persist pump duration reservation")
}

pub trait PumpDriver {
    fn pulse(&self, channel: u8, duty_percent: u8, duration: Duration) -> Result<()>;
}

pub fn execute(
    config: &Config,
    command: &PumpCommand,
    driver: &impl PumpDriver,
) -> Result<PumpOutcome> {
    execute_with_budget(config, command, driver, false)
}

/// Local CLI diagnostics bypass configured duration and aggregate limits in
/// debug mode. MQTT requests always use `execute` and keep those limits.
pub fn execute_local(
    config: &Config,
    command: &PumpCommand,
    driver: &impl PumpDriver,
) -> Result<PumpOutcome> {
    let local_debug = config.pump.as_ref().is_some_and(|pump| pump.debug_mode);
    execute_with_budget(config, command, driver, local_debug)
}

fn execute_with_budget(
    config: &Config,
    command: &PumpCommand,
    driver: &impl PumpDriver,
    local_debug: bool,
) -> Result<PumpOutcome> {
    let pump = validate_with_duration(config, command, local_debug)?;
    let _lock = PulseLock::acquire(&pump.state_file)?;
    let outcome = reserve(pump, command, now_ms()?, local_debug)?;
    if outcome == PumpOutcome::Duplicate {
        return Ok(outcome);
    }
    driver.pulse(
        command.channel,
        command.duty_percent,
        Duration::from_millis(command.duration_ms),
    )?;
    Ok(PumpOutcome::Completed)
}

#[cfg(target_os = "linux")]
pub struct GpioPumpDriver {
    pub chip: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl PumpDriver for GpioPumpDriver {
    fn pulse(&self, channel: u8, duty_percent: u8, duration: Duration) -> Result<()> {
        use gpiocdev::{Request, line::Value};
        use std::time::Instant;

        let pin = PINS[(channel - 1) as usize];
        let line = Request::builder()
            .on_chip(self.chip.clone())
            .with_line(pin)
            .as_output(Value::Inactive)
            .with_consumer("growhat-pump")
            .request()
            .context("request pump GPIO")?;
        struct OffOnDrop<'a>(&'a gpiocdev::Request, u32);
        impl Drop for OffOnDrop<'_> {
            fn drop(&mut self) {
                let _ = self.0.set_value(self.1, Value::Inactive);
            }
        }
        let _off = OffOnDrop(&line, pin);
        let high = PWM_PERIOD * u32::from(duty_percent) / 100;
        let low = PWM_PERIOD - high;
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            line.set_value(pin, Value::Active)?;
            std::thread::sleep(high.min(deadline.saturating_duration_since(Instant::now())));
            line.set_value(pin, Value::Inactive)?;
            let rest = deadline.saturating_duration_since(Instant::now());
            if rest.is_zero() {
                break;
            }
            std::thread::sleep(low.min(rest));
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
pub struct GpioPumpDriver {
    pub chip: std::path::PathBuf,
}

#[cfg(not(target_os = "linux"))]
impl PumpDriver for GpioPumpDriver {
    fn pulse(&self, _channel: u8, _duty_percent: u8, _duration: Duration) -> Result<()> {
        bail!("pump hardware requires Linux")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingDriver(AtomicUsize);
    impl PumpDriver for CountingDriver {
        fn pulse(&self, _channel: u8, _duty: u8, _duration: Duration) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn duplicate_and_budget_do_not_reactivate() {
        let path = std::env::temp_dir().join(format!(
            "growhat-pump-test-{}-{}",
            std::process::id(),
            now_ms().unwrap()
        ));
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.backend = crate::config::Backend::Hardware;
        config.hardware_confirmed = true;
        config.pump = Some(PumpConfig {
            enabled: true,
            channels: vec![1],
            max_duration_ms: 2_000,
            max_total_ms_per_24h: 2_000,
            debug_mode: false,
            state_file: path.clone(),
            mqtt_token_file: None,
        });
        let driver = CountingDriver(AtomicUsize::new(0));
        let mut command = PumpCommand {
            request_id: "first".into(),
            channel: 1,
            duty_percent: 50,
            duration_ms: 1_000,
        };
        assert_eq!(
            execute(&config, &command, &driver).unwrap(),
            PumpOutcome::Completed
        );
        assert_eq!(
            execute(&config, &command, &driver).unwrap(),
            PumpOutcome::Duplicate
        );
        command.request_id = "second".into();
        assert_eq!(
            execute(&config, &command, &driver).unwrap(),
            PumpOutcome::Completed
        );
        command.request_id = "third".into();
        assert!(execute(&config, &command, &driver).is_err());
        config.pump.as_mut().unwrap().debug_mode = true;
        assert!(execute(&config, &command, &driver).is_err());
        assert_eq!(
            execute_local(&config, &command, &driver).unwrap(),
            PumpOutcome::Completed
        );
        command.request_id = "fourth".into();
        assert!(execute(&config, &command, &driver).is_err());
        assert_eq!(driver.0.load(Ordering::SeqCst), 3);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("lock"));
    }

    #[test]
    fn invalid_and_corrupt_state_fail_before_gpio() {
        let path = std::env::temp_dir().join(format!(
            "growhat-pump-corrupt-test-{}-{}",
            std::process::id(),
            now_ms().unwrap()
        ));
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.backend = crate::config::Backend::Hardware;
        config.hardware_confirmed = true;
        config.pump = Some(PumpConfig {
            enabled: true,
            channels: vec![1],
            max_duration_ms: 2_000,
            max_total_ms_per_24h: 4_000,
            debug_mode: false,
            state_file: path.clone(),
            mqtt_token_file: None,
        });
        let driver = CountingDriver(AtomicUsize::new(0));
        let mut command = PumpCommand {
            request_id: "safe".into(),
            channel: 2,
            duty_percent: 50,
            duration_ms: 1_000,
        };
        assert!(execute(&config, &command, &driver).is_err());
        command.channel = 1;
        command.duty_percent = 0;
        assert!(execute(&config, &command, &driver).is_err());
        command.duty_percent = 50;
        command.duration_ms = 2_001;
        assert!(execute(&config, &command, &driver).is_err());
        command.duration_ms = 1_000;
        fs::write(&path, b"not JSON").unwrap();
        assert!(execute(&config, &command, &driver).is_err());
        assert_eq!(driver.0.load(Ordering::SeqCst), 0);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("lock"));
    }

    #[test]
    fn local_debug_ignores_configured_pulse_limit_but_has_finite_cutoff() {
        let path = std::env::temp_dir().join(format!(
            "growhat-pump-debug-test-{}-{}",
            std::process::id(),
            now_ms().unwrap()
        ));
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.backend = crate::config::Backend::Hardware;
        config.hardware_confirmed = true;
        config.pump = Some(PumpConfig {
            enabled: true,
            channels: vec![1],
            max_duration_ms: 2_000,
            max_total_ms_per_24h: 2_000,
            debug_mode: true,
            state_file: path.clone(),
            mqtt_token_file: None,
        });
        let driver = CountingDriver(AtomicUsize::new(0));
        let mut command = PumpCommand {
            request_id: "debug-long".into(),
            channel: 1,
            duty_percent: 90,
            duration_ms: 10_000,
        };
        assert!(execute(&config, &command, &driver).is_err());
        assert_eq!(
            execute_local(&config, &command, &driver).unwrap(),
            PumpOutcome::Completed
        );
        command.request_id = "too-long".into();
        command.duration_ms = MAX_DEBUG_PULSE_MS + 1;
        assert!(execute_local(&config, &command, &driver).is_err());
        assert_eq!(driver.0.load(Ordering::SeqCst), 1);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("lock"));
    }
}
