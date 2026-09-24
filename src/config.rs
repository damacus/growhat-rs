use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

fn sample_interval() -> u64 {
    5_000
}
fn stale_after() -> u64 {
    15_000
}
fn measurement_window() -> u64 {
    1_000
}
fn gpio_chip() -> PathBuf {
    "/dev/gpiochip0".into()
}
fn mqtt_port() -> u16 {
    1883
}
fn topic_prefix() -> String {
    "growhat".into()
}
fn discovery_prefix() -> String {
    "homeassistant".into()
}
fn simulated_hz() -> f64 {
    10.0
}
fn pump_max_duration() -> u64 {
    5_000
}
fn pump_daily_limit() -> u64 {
    30_000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Simulated,
    Hardware,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub dry_hz: f64,
    pub wet_hz: f64,
}

impl Calibration {
    pub fn moisture_percent(&self, hz: f64) -> f64 {
        ((hz - self.dry_hz) / (self.wet_hz - self.dry_hz) * 100.0).clamp(0.0, 100.0)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SensorConfig {
    pub channel: u8,
    pub name: String,
    pub calibration: Option<Calibration>,
    #[serde(default = "simulated_hz")]
    pub simulated_hz: f64,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MqttConfig {
    pub host: String,
    #[serde(default = "mqtt_port")]
    pub port: u16,
    #[serde(default = "topic_prefix")]
    pub topic_prefix: String,
    #[serde(default = "discovery_prefix")]
    pub discovery_prefix: String,
    pub username: Option<String>,
    pub password_file: Option<PathBuf>,
}

impl std::fmt::Debug for MqttConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MqttConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("topic_prefix", &self.topic_prefix)
            .field("discovery_prefix", &self.discovery_prefix)
            .field("username", &self.username.as_ref().map(|_| "[redacted]"))
            .field(
                "password_file",
                &self.password_file.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PumpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub channels: Vec<u8>,
    #[serde(default = "pump_max_duration")]
    pub max_duration_ms: u64,
    #[serde(default = "pump_daily_limit")]
    pub max_total_ms_per_24h: u64,
    /// Allow repeated local CLI diagnostics beyond the rolling daily budget.
    /// MQTT requests and the per-pulse maximum remain bounded.
    #[serde(default)]
    pub debug_mode: bool,
    pub state_file: PathBuf,
    pub mqtt_token_file: Option<PathBuf>,
}

impl std::fmt::Debug for PumpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PumpConfig")
            .field("enabled", &self.enabled)
            .field("channels", &self.channels)
            .field("max_duration_ms", &self.max_duration_ms)
            .field("max_total_ms_per_24h", &self.max_total_ms_per_24h)
            .field("debug_mode", &self.debug_mode)
            .field("state_file", &self.state_file)
            .field(
                "mqtt_token_file",
                &self.mqtt_token_file.as_ref().map(|_| "[redacted]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub device_id: String,
    pub name: String,
    pub backend: Backend,
    #[serde(default = "sample_interval")]
    pub sample_interval_ms: u64,
    #[serde(default = "stale_after")]
    pub stale_after_ms: u64,
    #[serde(default = "measurement_window")]
    pub measurement_window_ms: u64,
    #[serde(default = "gpio_chip")]
    pub gpio_chip: PathBuf,
    #[serde(default)]
    pub hardware_confirmed: bool,
    /// Enable the verified Grow HAT Mini display and four input buttons.
    #[serde(default)]
    pub panel_enabled: bool,
    pub sensors: Vec<SensorConfig>,
    pub mqtt: Option<MqttConfig>,
    pub pump: Option<PumpConfig>,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let body =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        // TOML parser diagnostics can echo source lines, including a rejected inline password.
        let mut config: Self = toml::from_str(&body)
            .map_err(|_| anyhow::anyhow!("invalid TOML configuration or unsupported field"))?;
        if let Some(mqtt) = &mut config.mqtt {
            if let Some(password_file) = &mut mqtt.password_file {
                if password_file.is_relative() {
                    *password_file = path
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(&*password_file);
                }
            }
        }
        if let Some(pump) = &mut config.pump {
            if pump.state_file.is_relative() {
                pump.state_file = path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(&pump.state_file);
            }
            if let Some(file) = &mut pump.mqtt_token_file {
                if file.is_relative() {
                    *file = path.parent().unwrap_or(Path::new(".")).join(&*file);
                }
            }
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_segment(&self.device_id) {
            bail!("device_id must contain only ASCII letters, digits, underscores or hyphens");
        }
        if self.name.trim().is_empty() {
            bail!("name must not be empty");
        }
        for (name, value) in [
            ("sample_interval_ms", self.sample_interval_ms),
            ("stale_after_ms", self.stale_after_ms),
            ("measurement_window_ms", self.measurement_window_ms),
        ] {
            if !(1..=3_600_000).contains(&value) {
                bail!("{name} must be between 1 and 3600000");
            }
        }
        if self.stale_after_ms < self.sample_interval_ms {
            bail!("stale_after_ms must be at least sample_interval_ms");
        }
        if self.measurement_window_ms > self.sample_interval_ms {
            bail!("measurement_window_ms must not exceed sample_interval_ms");
        }
        if self.sensors.is_empty() || self.sensors.len() > 3 {
            bail!("configure one to three sensors");
        }
        if matches!(self.backend, Backend::Hardware) && !self.hardware_confirmed {
            bail!("hardware backend requires hardware_confirmed = true");
        }
        if self.panel_enabled
            && (!self.hardware_confirmed || !matches!(self.backend, Backend::Hardware))
        {
            bail!("panel_enabled requires confirmed hardware backend");
        }
        if self.gpio_chip.as_os_str().is_empty() {
            bail!("gpio_chip must not be empty");
        }
        let mut seen = HashSet::new();
        for sensor in &self.sensors {
            if !(1..=3).contains(&sensor.channel) || !seen.insert(sensor.channel) {
                bail!("sensor channels must be unique and within 1..3");
            }
            if sensor.name.trim().is_empty() {
                bail!("sensor name must not be empty");
            }
            if !sensor.simulated_hz.is_finite() || sensor.simulated_hz <= 0.0 {
                bail!("simulated_hz must be positive and finite");
            }
            if let Some(c) = &sensor.calibration {
                if !c.dry_hz.is_finite()
                    || !c.wet_hz.is_finite()
                    || c.dry_hz <= 0.0
                    || c.wet_hz <= 0.0
                    || c.dry_hz == c.wet_hz
                {
                    bail!("calibration endpoints must be distinct positive finite frequencies");
                }
            }
        }
        if let Some(mqtt) = &self.mqtt {
            if mqtt.password_file.is_some() && mqtt.username.is_none() {
                bail!("MQTT password_file requires username");
            }
            if mqtt.host.trim().is_empty()
                || mqtt.host.contains(char::is_whitespace)
                || mqtt.port == 0
            {
                bail!("invalid MQTT host or port");
            }
            for prefix in [&mqtt.topic_prefix, &mqtt.discovery_prefix] {
                if !prefix.split('/').all(valid_segment) {
                    bail!("MQTT prefixes must contain safe nonempty topic segments");
                }
            }
        }
        if let Some(pump) = &self.pump {
            if pump.enabled {
                if !self.hardware_confirmed || !matches!(self.backend, Backend::Hardware) {
                    bail!("pump requires confirmed hardware backend");
                }
                if pump.channels.is_empty() || pump.channels.len() > 3 {
                    bail!("pump requires one to three enabled channels");
                }
                let mut channels = HashSet::new();
                if pump
                    .channels
                    .iter()
                    .any(|channel| !(1..=3).contains(channel) || !channels.insert(*channel))
                {
                    bail!("pump channels must be unique and within 1..3");
                }
                if !(1..=20_000).contains(&pump.max_duration_ms)
                    || !(pump.max_duration_ms..=300_000).contains(&pump.max_total_ms_per_24h)
                {
                    bail!("pump duration and 24-hour budget are out of range");
                }
                if pump.state_file.as_os_str().is_empty() || pump.state_file.file_name().is_none() {
                    bail!("pump state_file must name a file");
                }
                if pump.mqtt_token_file.is_some() && self.mqtt.is_none() {
                    bail!("pump mqtt_token_file requires MQTT configuration");
                }
            }
        }
        Ok(())
    }
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.is_ascii()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibration_directions_and_clamping() {
        for (dry, wet) in [(30.0, 1.0), (1.0, 30.0)] {
            let c = Calibration {
                dry_hz: dry,
                wet_hz: wet,
            };
            assert_eq!(c.moisture_percent(dry), 0.0);
            assert_eq!(c.moisture_percent(wet), 100.0);
            assert_eq!(c.moisture_percent((dry + wet) / 2.0), 50.0);
            assert!((0.0..=100.0).contains(&c.moisture_percent(wet + (wet - dry))));
        }
    }
    #[test]
    fn rejects_unknown_and_unconfirmed_hardware() {
        let body = "device_id='plant_1'\nname='Plant'\nbackend='hardware'\nunknown=2\n[[sensors]]\nchannel=1\nname='Soil'";
        assert!(toml::from_str::<Config>(body).is_err());
        let body = body.replace("unknown=2\n", "");
        assert!(toml::from_str::<Config>(&body).unwrap().validate().is_err());
    }
    #[test]
    fn rejects_duplicate_channels_bad_timings_and_nonfinite_calibration() {
        let body = "device_id='plant_1'\nname='Plant'\nbackend='simulated'\n[[sensors]]\nchannel=1\nname='Soil'";
        let mut cfg: Config = toml::from_str(body).unwrap();
        cfg.sensors.push(cfg.sensors[0].clone());
        assert!(cfg.validate().is_err());
        cfg.sensors.pop();
        cfg.sample_interval_ms = 0;
        assert!(cfg.validate().is_err());
        cfg.sample_interval_ms = 5_000;
        cfg.measurement_window_ms = 5_001;
        assert!(cfg.validate().is_err());
        cfg.measurement_window_ms = 1_000;
        cfg.sensors[0].calibration = Some(Calibration {
            dry_hz: f64::NAN,
            wet_hz: 1.0,
        });
        assert!(cfg.validate().is_err());
        cfg.sensors[0].calibration = Some(Calibration {
            dry_hz: 1.0,
            wet_hz: 1.0,
        });
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_password_file_without_username() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.mqtt = Some(MqttConfig {
            host: "localhost".into(),
            port: 1883,
            topic_prefix: "growhat".into(),
            discovery_prefix: "homeassistant".into(),
            username: None,
            password_file: Some("secret".into()),
        });
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("requires username")
        );
    }

    #[test]
    fn panel_requires_confirmed_hardware() {
        let mut config = Config::load("examples/simulated.toml").unwrap();
        config.panel_enabled = true;
        assert!(config.validate().is_err());
        config.backend = Backend::Hardware;
        assert!(config.validate().is_err());
        config.hardware_confirmed = true;
        assert!(config.validate().is_ok());
    }
}
