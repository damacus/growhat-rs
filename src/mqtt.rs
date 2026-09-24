use anyhow::{Context, Result, bail};
use growhat::{
    config::Config,
    panel::{ButtonEvent, PanelCommand},
    pump::{self, GpioPumpDriver, PumpCommand, PumpOutcome},
    sensor::{ReadingStatus, SensorReading},
};
use rumqttc::{AsyncClient, Event, Incoming, LastWill, MqttOptions, Outgoing, QoS};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

fn display_command(payload: &[u8]) -> Result<PanelCommand> {
    if payload.len() > 512 {
        bail!("command too large");
    }
    let value: Value = serde_json::from_slice(payload).context("invalid command JSON")?;
    let object = value.as_object().context("command must be an object")?;
    if object
        .keys()
        .any(|key| key != "text" && key != "ttl_seconds")
    {
        bail!("unknown command field");
    }
    let text = object
        .get("text")
        .and_then(Value::as_str)
        .context("text must be a string")?;
    if text.is_empty()
        || text.len() > 120
        || !text
            .bytes()
            .all(|b| b == b'\n' || (0x20..=0x7e).contains(&b))
    {
        bail!("text must be 1..120 printable ASCII bytes or newlines");
    }
    let ttl_seconds = match object.get("ttl_seconds") {
        Some(value) => value
            .as_u64()
            .context("ttl_seconds must be an unsigned integer")?,
        None => 30,
    };
    if !(1..=300).contains(&ttl_seconds) {
        bail!("ttl_seconds must be between 1 and 300");
    }
    Ok(PanelCommand::Message {
        text: text.into(),
        ttl_seconds,
    })
}

fn button_discovery(config: &Config) -> Vec<(String, Value)> {
    if !config.panel_enabled {
        return Vec::new();
    }
    let base = base(config);
    let prefix = &config.mqtt.as_ref().unwrap().discovery_prefix;
    let mut messages = Vec::new();
    for button in ["A", "B", "X", "Y"] {
        let lower = button.to_ascii_lowercase();
        let state_topic = format!("{base}/buttons/{button}/state");
        let device = json!({"identifiers": [config.device_id], "name": config.name,
            "manufacturer": "Pimoroni", "model": "Grow HAT Mini", "sw_version": env!("CARGO_PKG_VERSION")});
        messages.push((format!("{prefix}/binary_sensor/{}/button_{}_held/config", config.device_id, lower), json!({
            "name": format!("Button {button} held"), "unique_id": format!("{}_button_{}_held", config.device_id, lower),
            "state_topic": state_topic, "value_template": "{{ 'ON' if value_json.pressed else 'OFF' }}",
            "availability_topic": format!("{base}/availability"), "device": device,
            "entity_category": "diagnostic"
        })));
        messages.push((format!("{prefix}/sensor/{}/button_{}_presses/config", config.device_id, lower), json!({
            "name": format!("Button {button} presses"), "unique_id": format!("{}_button_{}_presses", config.device_id, lower),
            "state_topic": state_topic, "value_template": "{{ value_json.count }}", "state_class": "total_increasing",
            "availability_topic": format!("{base}/availability"), "device": device,
            "entity_category": "diagnostic"
        })));
        messages.push((
            format!(
                "{prefix}/button/{}/button_{}_remote/config",
                config.device_id, lower
            ),
            json!({
                "name": format!("Press {button} on HAT"),
                "unique_id": format!("{}_button_{}_remote", config.device_id, lower),
                "command_topic": format!("{base}/buttons/{button}/press/set"),
                "payload_press": "PRESS", "retain": false,
                "availability_topic": format!("{base}/availability"), "device": device
            }),
        ));
    }
    messages
}

#[derive(Clone)]
pub struct Batch {
    pub readings: Vec<SensorReading>,
    pub captured_at: Instant,
}

pub fn current_readings(batch: &Batch, stale_after_ms: u64) -> Vec<SensorReading> {
    age_readings(batch, batch.captured_at.elapsed(), stale_after_ms)
}

fn age_readings(batch: &Batch, elapsed: Duration, stale_after_ms: u64) -> Vec<SensorReading> {
    let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
    batch
        .readings
        .iter()
        .cloned()
        .map(|mut reading| {
            reading.age_ms = reading.age_ms.map(|age| age.saturating_add(elapsed_ms));
            if elapsed_ms >= stale_after_ms {
                reading.status = ReadingStatus::Stale;
                reading.moisture_percent = None;
                reading.error = Some("last sensor snapshot is stale".into());
            }
            reading
        })
        .collect()
}

fn base(config: &Config) -> String {
    format!(
        "{}/{}",
        config.mqtt.as_ref().unwrap().topic_prefix,
        config.device_id
    )
}

pub fn discovery(config: &Config) -> Vec<(String, Value)> {
    let mqtt = config
        .mqtt
        .as_ref()
        .expect("MQTT configuration validated by caller");
    let base = base(config);
    let mut messages = Vec::new();
    for sensor in &config.sensors {
        // Earlier raw entities were diagnostic; retire their discovery identities
        // so the user-facing frequency entities start in Home Assistant's Sensors list.
        messages.push((
            format!(
                "{}/sensor/{}_{}/raw/config",
                mqtt.discovery_prefix, config.device_id, sensor.channel
            ),
            Value::Null,
        ));
        for kind in ["frequency", "moisture", "status"] {
            let topic = format!(
                "{}/sensor/{}_{}/{kind}/config",
                mqtt.discovery_prefix, config.device_id, sensor.channel
            );
            // Remove a previous calibrated entity when its calibration is removed.
            if kind == "moisture" && sensor.calibration.is_none() {
                messages.push((topic, Value::Null));
                continue;
            }
            let state_topic = format!("{base}/{}/state", sensor.channel);
            let mut payload = json!({
                "name": if kind == "frequency" {
                    format!("{} frequency", sensor.name)
                } else {
                    format!("{} {}", sensor.name, kind)
                },
                "unique_id": format!("{}_{}_{}", config.device_id, sensor.channel, kind),
                "state_topic": state_topic,
                "json_attributes_topic": state_topic,
                "expire_after": config.stale_after_ms.div_ceil(1000),
                "device": {
                    "identifiers": [config.device_id], "name": config.name,
                    "manufacturer": "Pimoroni", "model": "Grow HAT Mini",
                    "sw_version": env!("CARGO_PKG_VERSION")
                },
                "availability": [{"topic": format!("{base}/availability")}],
                "availability_mode": "all"
            });
            if kind != "status" {
                payload["availability"].as_array_mut().unwrap().push(json!({
                    "topic": state_topic,
                    "value_template": "{{ 'online' if value_json.status in ['valid', 'uncalibrated'] else 'offline' }}"
                }));
                payload["state_class"] = json!("measurement");
            }
            match kind {
                "frequency" => {
                    payload["value_template"] = json!("{{ value_json.raw_hz }}");
                    payload["unit_of_measurement"] = json!("Hz");
                    payload["device_class"] = json!("frequency");
                    payload["suggested_display_precision"] = json!(2);
                }
                "moisture" => {
                    payload["value_template"] = json!("{{ value_json.moisture_percent }}");
                    payload["unit_of_measurement"] = json!("%");
                    payload["device_class"] = json!("moisture");
                }
                _ => {
                    payload["value_template"] = json!("{{ value_json.status }}");
                    payload["entity_category"] = json!("diagnostic");
                }
            }
            messages.push((topic, payload));
        }
    }
    messages.extend(button_discovery(config));
    messages
}

fn announce(client: &AsyncClient, config: &Config) -> Result<()> {
    for (topic, payload) in discovery(config) {
        let body = if payload.is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(&payload)?
        };
        client.try_publish(topic, QoS::AtLeastOnce, true, body)?;
    }
    client.try_publish(
        format!("{}/availability", base(config)),
        QoS::AtLeastOnce,
        true,
        "online",
    )?;
    Ok(())
}

fn publish_readings(
    client: &AsyncClient,
    config: &Config,
    batch: &Batch,
) -> Result<Vec<ReadingStatus>> {
    let readings = current_readings(batch, config.stale_after_ms);
    for reading in &readings {
        client.try_publish(
            format!("{}/{}/state", base(config), reading.channel),
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(reading)?,
        )?;
    }
    Ok(readings.iter().map(|r| r.status).collect())
}

fn publish_button(client: &AsyncClient, config: &Config, event: &ButtonEvent) -> Result<()> {
    if !["A", "B", "X", "Y"].contains(&event.button.as_str()) {
        bail!("invalid button event");
    }
    client.try_publish(
        format!("{}/buttons/{}/state", base(config), event.button),
        QoS::AtMostOnce,
        false,
        serde_json::to_vec(event)?,
    )?;
    Ok(())
}

fn drain_button_events(
    events: &mut mpsc::Receiver<ButtonEvent>,
    states: &mut std::collections::BTreeMap<String, ButtonEvent>,
    open: &mut bool,
) {
    loop {
        match events.try_recv() {
            Ok(event) => {
                states.insert(event.button.clone(), event);
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                *open = false;
                break;
            }
        }
    }
}

fn handle_display(
    client: &AsyncClient,
    config: &Config,
    payload: &[u8],
    retain: bool,
    panel_commands: &Option<SyncSender<PanelCommand>>,
) -> Result<()> {
    let (accepted, text, reason) = match display_command(payload) {
        Ok(command @ PanelCommand::Message { .. }) => {
            let text = match &command {
                PanelCommand::Message { text, .. } => text.clone(),
                _ => unreachable!(),
            };
            if retain {
                (
                    false,
                    Some(text),
                    Some("retained commands are not accepted".to_owned()),
                )
            } else if !config.panel_enabled {
                (false, Some(text), Some("panel disabled".to_owned()))
            } else if !growhat::panel::message_fits(&text) {
                (
                    false,
                    Some(text),
                    Some("text exceeds display capacity".to_owned()),
                )
            } else {
                match panel_commands
                    .as_ref()
                    .map(|sender| sender.try_send(command))
                {
                    Some(Ok(())) => (true, Some(text), None),
                    _ => (
                        false,
                        Some(text),
                        Some("panel queue unavailable".to_owned()),
                    ),
                }
            }
        }
        Ok(_) => unreachable!(),
        Err(error) => (false, None, Some(error.to_string())),
    };
    eprintln!(
        "MQTT display command {}: {}",
        if accepted { "queued" } else { "rejected" },
        reason.as_deref().unwrap_or("accepted")
    );
    client.try_publish(
        format!("{}/display/ack", base(config)),
        QoS::AtMostOnce,
        false,
        serde_json::to_vec(
            &json!({"accepted": accepted, "status": if accepted { "queued" } else { "rejected" },
            "text": text, "reason": reason}),
        )?,
    )?;
    Ok(())
}

fn handle_virtual_press(
    client: &AsyncClient,
    config: &Config,
    button: &str,
    payload: &[u8],
    retain: bool,
    panel_commands: &Option<SyncSender<PanelCommand>>,
) -> Result<()> {
    let reason = if !["A", "B", "X", "Y"].contains(&button) {
        Some("unknown button")
    } else if retain || payload != b"PRESS" {
        Some("retained or invalid button command")
    } else if !config.panel_enabled {
        Some("panel disabled")
    } else {
        match panel_commands.as_ref().map(|sender| {
            sender.try_send(PanelCommand::VirtualPress {
                button: button.into(),
            })
        }) {
            Some(Ok(())) => None,
            _ => Some("panel queue unavailable"),
        }
    };
    client.try_publish(
        format!("{}/buttons/{button}/press/ack", base(config)),
        QoS::AtMostOnce,
        false,
        serde_json::to_vec(&json!({"accepted": reason.is_none(), "reason": reason}))?,
    )?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PumpMqttRequest {
    request_id: String,
    channel: u8,
    duty_percent: u8,
    duration_ms: u64,
    token: String,
}

fn pump_ack(
    client: &AsyncClient,
    config: &Config,
    request_id: Option<&str>,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    client.try_publish(
        format!("{}/pump/ack", base(config)),
        QoS::AtMostOnce,
        false,
        serde_json::to_vec(&json!({"request_id": request_id, "status": status, "reason": reason}))?,
    )?;
    Ok(())
}

fn pump_token_valid(config: &Config, token: &str) -> bool {
    let Some(path) = config
        .pump
        .as_ref()
        .and_then(|pump| pump.mqtt_token_file.as_ref())
    else {
        return false;
    };
    let Ok(expected) = std::fs::read_to_string(path) else {
        return false;
    };
    let expected = expected.trim_end_matches(['\r', '\n']);
    if expected.len() < 32 || expected.len() != token.len() {
        return false;
    }
    expected
        .bytes()
        .zip(token.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

fn handle_pump(client: &AsyncClient, config: &Config, payload: &[u8], retain: bool) -> Result<()> {
    if retain || payload.len() > 512 {
        return pump_ack(
            client,
            config,
            None,
            "rejected",
            Some("retained or oversized command"),
        );
    }
    let request: PumpMqttRequest = match serde_json::from_slice(payload) {
        Ok(request) => request,
        Err(_) => {
            return pump_ack(
                client,
                config,
                None,
                "rejected",
                Some("invalid command JSON"),
            );
        }
    };
    let command = PumpCommand {
        request_id: request.request_id,
        channel: request.channel,
        duty_percent: request.duty_percent,
        duration_ms: request.duration_ms,
    };
    if !pump_token_valid(config, &request.token) {
        return pump_ack(
            client,
            config,
            Some(&command.request_id),
            "rejected",
            Some("unauthorised"),
        );
    }
    if let Err(error) = pump::validate(config, &command) {
        return pump_ack(
            client,
            config,
            Some(&command.request_id),
            "rejected",
            Some(&error.to_string()),
        );
    }
    let client = client.clone();
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let driver = GpioPumpDriver {
            chip: config.gpio_chip.clone(),
        };
        let result = pump::execute(&config, &command, &driver);
        let (status, reason) = match result {
            Ok(PumpOutcome::Completed) => ("completed", None),
            Ok(PumpOutcome::Duplicate) => ("duplicate", None),
            Err(error) => {
                eprintln!("pump request rejected or failed: {error:#}");
                ("rejected", Some(error.to_string()))
            }
        };
        if let Err(error) = pump_ack(
            &client,
            &config,
            Some(&command.request_id),
            status,
            reason.as_deref(),
        ) {
            eprintln!("pump acknowledgement unavailable: {error}");
        }
    });
    Ok(())
}

fn options(config: &Config) -> Result<MqttOptions> {
    let mqtt = config
        .mqtt
        .as_ref()
        .context("run requires an [mqtt] section; use diagnose for readings without MQTT")?;
    let mut options = MqttOptions::new(
        format!("growhat_{}", config.device_id),
        &mqtt.host,
        mqtt.port,
    );
    options.set_keep_alive(Duration::from_secs(5));
    options.set_clean_session(true);
    options.set_last_will(LastWill::new(
        format!("{}/availability", base(config)),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));
    if let Some(username) = &mqtt.username {
        let password = match &mqtt.password_file {
            Some(path) => std::fs::read_to_string(path)
                .context("cannot read MQTT password_file; check its path and permissions")?,
            None => String::new(),
        };
        options.set_credentials(username, password.trim_end_matches(['\r', '\n']));
    } else if mqtt.password_file.is_some() {
        bail!("MQTT password_file requires username");
    }
    Ok(options)
}

pub async fn serve(
    config: &Config,
    mut batches: watch::Receiver<Batch>,
    mut stop: watch::Receiver<bool>,
    panel_commands: Option<SyncSender<PanelCommand>>,
    mut button_events: mpsc::Receiver<ButtonEvent>,
) -> Result<()> {
    let options = options(config)?;
    let birth_topic = format!("{}/status", config.mqtt.as_ref().unwrap().discovery_prefix);
    let display_topic = format!("{}/display/set", base(config));
    let pump_topic = format!("{}/pump/set", base(config));
    let virtual_button_topic = format!("{}/buttons/+/press/set", base(config));
    let virtual_button_prefix = format!("{}/buttons/", base(config));
    let mut button_states = std::collections::BTreeMap::<String, ButtonEvent>::new();
    let mut button_events_open = true;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        // A new client on each connection failure drops queued telemetry. Only
        // the watch channel's newest measurement survives an outage.
        let (client, mut eventloop) = AsyncClient::new(options.clone(), 64);
        eventloop.network_options.set_connection_timeout(5);
        let mut connected = false;
        let mut last_health = Vec::new();
        let mut health_tick = tokio::time::interval(Duration::from_secs(1));
        health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    if connected {
                        let _ = client.try_publish(format!("{}/availability", base(config)), QoS::AtLeastOnce, true, "offline");
                        let _ = client.try_disconnect();
                        let _ = tokio::time::timeout(Duration::from_secs(2), async {
                            loop {
                                match eventloop.poll().await {
                                    Ok(Event::Outgoing(Outgoing::Disconnect)) | Err(_) => break,
                                    _ => {}
                                }
                            }
                        }).await;
                    }
                    return Ok(());
                }
                event = eventloop.poll() => {
                    match event {
                        Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                            connected = true;
                            if config.panel_enabled {
                                drain_button_events(&mut button_events, &mut button_states, &mut button_events_open);
                            }
                            eprintln!("MQTT connected");
                            if let Err(error) = client.try_subscribe(&birth_topic, QoS::AtMostOnce).map_err(anyhow::Error::from)
                                .and_then(|_| client.try_subscribe(&display_topic, QoS::AtMostOnce).map_err(anyhow::Error::from))
                                .and_then(|_| client.try_subscribe(&pump_topic, QoS::AtMostOnce).map_err(anyhow::Error::from))
                                .and_then(|_| client.try_subscribe(&virtual_button_topic, QoS::AtMostOnce).map_err(anyhow::Error::from))
                                .and_then(|_| announce(&client, config))
                                .and_then(|_| publish_readings(&client, config, &batches.borrow()))
                                .and_then(|health| {
                                    for event in button_states.values() { publish_button(&client, config, event)?; }
                                    Ok(health)
                                })
                                .map(|health| last_health = health)
                            {
                                eprintln!("MQTT request queue unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(message)))
                            if message.topic == birth_topic && message.payload.as_ref() == b"online" => {
                            if let Err(error) = announce(&client, config)
                                .and_then(|_| publish_readings(&client, config, &batches.borrow()))
                                .and_then(|health| {
                                    for event in button_states.values() { publish_button(&client, config, event)?; }
                                    Ok(health)
                                })
                                .map(|health| last_health = health)
                            {
                                eprintln!("MQTT request queue unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(message))) if message.topic == display_topic => {
                            if let Err(error) = handle_display(&client, config, &message.payload, message.retain, &panel_commands) {
                                eprintln!("MQTT request queue unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(message))) if message.topic == pump_topic => {
                            if let Err(error) = handle_pump(&client, config, &message.payload, message.retain) {
                                eprintln!("MQTT pump acknowledgement unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                        Ok(Event::Incoming(Incoming::Publish(message)))
                            if message.topic.starts_with(&virtual_button_prefix)
                                && message.topic.ends_with("/press/set") => {
                            let button = message.topic
                                .strip_prefix(&virtual_button_prefix)
                                .and_then(|part| part.strip_suffix("/press/set"))
                                .unwrap_or("");
                            if let Err(error) = handle_virtual_press(
                                &client, config, button, &message.payload, message.retain, &panel_commands,
                            ) {
                                eprintln!("MQTT virtual button acknowledgement unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("MQTT disconnected: {error}; retrying in 1s");
                            break;
                        }
                    }
                }
                changed = batches.changed(), if connected => {
                    changed.context("sensor worker stopped unexpectedly")?;
                    match publish_readings(&client, config, &batches.borrow()) {
                        Ok(health) => last_health = health,
                        Err(error) => {
                            eprintln!("MQTT request queue unavailable: {error}; retrying in 1s");
                            break;
                        }
                    }
                }
                _ = health_tick.tick(), if connected => {
                    let health: Vec<_> = current_readings(&batches.borrow(), config.stale_after_ms).iter().map(|r| r.status).collect();
                    if health != last_health {
                        match publish_readings(&client, config, &batches.borrow()) {
                            Ok(health) => last_health = health,
                            Err(error) => {
                                eprintln!("MQTT request queue unavailable: {error}; retrying in 1s");
                                break;
                            }
                        }
                    }
                }
                event = button_events.recv(), if connected && button_events_open => {
                    match event {
                        Some(event) if config.panel_enabled => {
                            button_states.insert(event.button.clone(), event.clone());
                            if connected {
                                if let Err(error) = publish_button(&client, config, &event) {
                                    eprintln!("MQTT button publish unavailable: {error}; retrying in 1s");
                                    break;
                                }
                            }
                        }
                        Some(_) => {}
                        None => button_events_open = false,
                    }
                }
            }
        }
        tokio::select! {
            _ = stop.changed() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_commands_enforce_json_and_bounds() {
        assert!(matches!(
            display_command(br#"{"text":"Water me"}"#).unwrap(),
            PanelCommand::Message {
                ttl_seconds: 30,
                ..
            }
        ));
        for payload in [
            br#"{"text":""}"#.as_slice(),
            br#"{"text":"ok","ttl_seconds":0}"#,
            br#"{"text":"ok","ttl_seconds":301}"#,
            br#"{"text":"ok","extra":1}"#,
            br#"{"text":"bad\ttext"}"#,
            br#"{"text":"\u00e9"}"#,
            b"not JSON",
        ] {
            assert!(display_command(payload).is_err());
        }
        let too_long = format!("{{\"text\":\"{}\"}}", "x".repeat(121));
        assert!(display_command(too_long.as_bytes()).is_err());
        let escaped = "\\\"".repeat(60);
        let payload = serde_json::to_vec(&json!({"text": escaped, "ttl_seconds": 30})).unwrap();
        assert!(payload.len() > 256);
        assert!(
            matches!(display_command(&payload).unwrap(), PanelCommand::Message { text, .. } if text.len() == 120)
        );
        assert!(display_command(&vec![b' '; 513]).is_err());
    }

    #[test]
    fn display_ack_only_accepts_queued_nonretained_command() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.panel_enabled = true;
        config.mqtt = Some(growhat::config::MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: None,
        });
        let (client, _eventloop) = AsyncClient::new(options(&config).unwrap(), 8);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let sender = Some(sender);
        handle_display(&client, &config, br#"{"text":"Water me"}"#, false, &sender).unwrap();
        assert!(matches!(
            receiver.try_recv().unwrap(),
            PanelCommand::Message {
                ttl_seconds: 30,
                ..
            }
        ));
        handle_display(&client, &config, br#"{"text":"ignored"}"#, true, &sender).unwrap();
        assert!(receiver.try_recv().is_err());
        let overflow =
            serde_json::to_vec(&json!({"text": format!("\n{}", "x".repeat(119))})).unwrap();
        handle_display(&client, &config, &overflow, false, &sender).unwrap();
        assert!(receiver.try_recv().is_err());
        handle_display(&client, &config, br#"{"text":"first"}"#, false, &sender).unwrap();
        handle_display(&client, &config, br#"{"text":"second"}"#, false, &sender).unwrap();
        assert!(
            matches!(receiver.try_recv().unwrap(), PanelCommand::Message { text, .. } if text == "first")
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn remote_button_only_queues_nonretained_press() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.panel_enabled = true;
        config.mqtt = Some(growhat::config::MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: None,
        });
        let (client, _eventloop) = AsyncClient::new(options(&config).unwrap(), 8);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let sender = Some(sender);
        handle_virtual_press(&client, &config, "A", b"PRESS", false, &sender).unwrap();
        assert!(
            matches!(receiver.try_recv().unwrap(), PanelCommand::VirtualPress { button } if button == "A")
        );
        handle_virtual_press(&client, &config, "A", b"PRESS", true, &sender).unwrap();
        handle_virtual_press(&client, &config, "A", b"press", false, &sender).unwrap();
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn reconnect_coalesces_button_backlog_to_latest_state() {
        let (sender, mut receiver) = mpsc::channel(4);
        for (pressed, count) in [(true, 1), (false, 1), (true, 2)] {
            sender
                .try_send(ButtonEvent {
                    button: "A".into(),
                    pressed,
                    count,
                })
                .unwrap();
        }
        sender
            .try_send(ButtonEvent {
                button: "B".into(),
                pressed: false,
                count: 0,
            })
            .unwrap();
        let mut states = std::collections::BTreeMap::new();
        let mut open = true;
        drain_button_events(&mut receiver, &mut states, &mut open);
        assert_eq!(states.len(), 2);
        assert_eq!(states["A"].count, 2);
        assert!(states["A"].pressed);
        assert!(!states["B"].pressed);
        assert!(open);
    }

    #[test]
    fn delayed_snapshot_becomes_stale_before_republish() {
        let batch = Batch {
            captured_at: Instant::now(),
            readings: vec![SensorReading {
                channel: 1,
                raw_hz: Some(10.0),
                moisture_percent: Some(50.0),
                status: ReadingStatus::Valid,
                age_ms: Some(500),
                error: None,
            }],
        };
        let readings = age_readings(&batch, Duration::from_millis(2000), 2000);
        assert_eq!(readings[0].status, ReadingStatus::Stale);
        assert_eq!(readings[0].moisture_percent, None);
        assert_eq!(readings[0].age_ms, Some(2500));
    }

    #[test]
    fn fresh_no_signal_snapshot_does_not_turn_stale_with_old_raw_value() {
        let batch = Batch {
            captured_at: Instant::now(),
            readings: vec![SensorReading {
                channel: 3,
                raw_hz: Some(4.0),
                moisture_percent: None,
                status: ReadingStatus::NoSignal,
                age_ms: Some(20_000),
                error: Some("no moisture pulses in measurement window".into()),
            }],
        };
        let readings = age_readings(&batch, Duration::from_millis(100), 15_000);
        assert_eq!(readings[0].status, ReadingStatus::NoSignal);
        assert_eq!(readings[0].age_ms, Some(20_100));
    }

    #[test]
    fn discovery_has_stable_identity_and_no_uncalibrated_percentage() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.mqtt = Some(growhat::config::MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: None,
        });
        let messages = discovery(&config);
        assert_eq!(messages.len(), 4);
        assert!(
            messages
                .iter()
                .find(|(t, _)| t.ends_with("/moisture/config"))
                .unwrap()
                .1
                .is_null()
        );
        assert!(
            messages
                .iter()
                .find(|(t, _)| t.ends_with("/raw/config"))
                .unwrap()
                .1
                .is_null()
        );
        let frequency = &messages
            .iter()
            .find(|(t, _)| t.ends_with("/frequency/config"))
            .unwrap()
            .1;
        assert_eq!(frequency["unique_id"], "growhat_demo_1_frequency");
        assert_eq!(frequency["availability"].as_array().unwrap().len(), 2);
        assert_eq!(frequency["availability_mode"], "all");
        assert!(frequency["expire_after"].as_u64().unwrap() > 0);
    }

    #[test]
    fn panel_discovery_reuses_existing_button_topics() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.panel_enabled = true;
        config.mqtt = Some(growhat::config::MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: None,
        });
        let messages = discovery(&config);
        for button in ["a", "b", "x", "y"] {
            let held = format!(
                "homeassistant/binary_sensor/{}/button_{button}_held/config",
                config.device_id
            );
            let presses = format!(
                "homeassistant/sensor/{}/button_{button}_presses/config",
                config.device_id
            );
            let remote = format!(
                "homeassistant/button/{}/button_{button}_remote/config",
                config.device_id
            );
            assert!(messages.iter().any(|(topic, payload)| topic == &held
                && payload["unique_id"] == format!("{}_button_{button}_held", config.device_id)));
            assert!(messages.iter().any(|(topic, payload)| topic == &presses
                && payload["unique_id"]
                    == format!("{}_button_{button}_presses", config.device_id)));
            assert!(messages.iter().any(|(topic, payload)| topic == &remote
                && payload["command_topic"]
                    == format!(
                        "growhat/{}/buttons/{}/press/set",
                        config.device_id,
                        button.to_ascii_uppercase()
                    )
                && payload["retain"] == false));
        }
    }

    #[test]
    fn full_rumqttc_queue_rejects_snapshot_without_waiting() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.mqtt = Some(growhat::config::MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: None,
        });
        let (client, _eventloop) = AsyncClient::new(options(&config).unwrap(), 1);
        client
            .try_publish("growhat/fill", QoS::AtMostOnce, false, "fill")
            .unwrap();
        let batch = Batch {
            captured_at: Instant::now(),
            readings: vec![SensorReading {
                channel: 1,
                raw_hz: Some(10.0),
                moisture_percent: None,
                status: ReadingStatus::Uncalibrated,
                age_ms: Some(0),
                error: None,
            }],
        };
        assert!(publish_readings(&client, &config, &batch).is_err());
    }
}
