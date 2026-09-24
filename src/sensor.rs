use crate::config::{Calibration, Config};
use serde::Serialize;
use std::{collections::HashMap, time::Duration};

pub trait SensorSource: Send {
    fn read(&mut self) -> Vec<(u8, Result<f64, SensorError>)>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SensorError {
    NoPulses,
    InsufficientPulses,
    Other(String),
}

impl std::fmt::Display for SensorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPulses => write!(f, "no moisture pulses in measurement window"),
            Self::InsufficientPulses => write!(f, "insufficient pulses to determine frequency"),
            Self::Other(error) => f.write_str(error),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadingStatus {
    Valid,
    Uncalibrated,
    NoSignal,
    Invalid,
    Stale,
}

#[derive(Clone, Debug, Serialize)]
pub struct SensorReading {
    pub channel: u8,
    pub raw_hz: Option<f64>,
    pub moisture_percent: Option<f64>,
    pub status: ReadingStatus,
    pub age_ms: Option<u64>,
    pub error: Option<String>,
}

pub struct SensorTracker {
    channels: Vec<(u8, Option<Calibration>)>,
    stale_after: Duration,
    last: HashMap<u8, (Duration, f64)>,
}

impl SensorTracker {
    pub fn new(config: &Config) -> Self {
        Self {
            channels: config
                .sensors
                .iter()
                .map(|s| (s.channel, s.calibration.clone()))
                .collect(),
            stale_after: Duration::from_millis(config.stale_after_ms),
            last: HashMap::new(),
        }
    }

    pub fn update(
        &mut self,
        samples: Vec<(u8, Result<f64, SensorError>)>,
        now: Duration,
    ) -> Vec<SensorReading> {
        let mut incoming = HashMap::new();
        for (channel, result) in samples {
            incoming.insert(channel, result);
        }
        self.channels
            .iter()
            .map(|(channel, calibration)| {
                let sample = incoming.remove(channel);
                match sample {
                    Some(Ok(hz)) if hz.is_finite() && hz > 0.0 => {
                        self.last.insert(*channel, (now, hz));
                        SensorReading {
                            channel: *channel,
                            raw_hz: Some(hz),
                            moisture_percent: calibration.as_ref().map(|c| c.moisture_percent(hz)),
                            status: if calibration.is_some() {
                                ReadingStatus::Valid
                            } else {
                                ReadingStatus::Uncalibrated
                            },
                            age_ms: Some(0),
                            error: None,
                        }
                    }
                    other => {
                        let age = self
                            .last
                            .get(channel)
                            .map(|(time, _)| now.saturating_sub(*time));
                        let stale = age.is_some_and(|a| a >= self.stale_after);
                        SensorReading {
                            channel: *channel,
                            raw_hz: self.last.get(channel).map(|(_, hz)| *hz),
                            moisture_percent: None,
                            status: match &other {
                                Some(Err(SensorError::NoPulses)) => ReadingStatus::NoSignal,
                                None if stale => ReadingStatus::Stale,
                                _ => ReadingStatus::Invalid,
                            },
                            age_ms: age.map(|a| a.as_millis().min(u64::MAX as u128) as u64),
                            error: Some(match other {
                                Some(Err(error)) => error.to_string(),
                                Some(Ok(_)) => "nonpositive or nonfinite frequency".into(),
                                None => "missing sample".into(),
                            }),
                        }
                    }
                }
            })
            .collect()
    }
}

pub struct SimulatedSource {
    samples: Vec<(u8, f64)>,
}
impl SimulatedSource {
    pub fn new(config: &Config) -> Self {
        Self {
            samples: config
                .sensors
                .iter()
                .map(|s| (s.channel, s.simulated_hz))
                .collect(),
        }
    }
}
impl SensorSource for SimulatedSource {
    fn read(&mut self) -> Vec<(u8, Result<f64, SensorError>)> {
        self.samples
            .iter()
            .map(|(channel, hz)| (*channel, Ok(*hz)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Backend, SensorConfig};
    #[test]
    fn invalid_then_stale_never_reuses_moisture() {
        let cfg = Config {
            device_id: "x".into(),
            name: "X".into(),
            backend: Backend::Simulated,
            sample_interval_ms: 1000,
            stale_after_ms: 2000,
            measurement_window_ms: 1000,
            gpio_chip: "/dev/gpiochip0".into(),
            hardware_confirmed: false,
            panel_enabled: false,
            pump: None,
            sensors: vec![SensorConfig {
                channel: 1,
                name: "Soil".into(),
                calibration: Some(Calibration {
                    dry_hz: 30.0,
                    wet_hz: 1.0,
                }),
                simulated_hz: 10.0,
            }],
            mqtt: None,
        };
        let mut tracker = SensorTracker::new(&cfg);
        assert_eq!(
            tracker.update(vec![(1, Ok(10.0))], Duration::ZERO)[0].status,
            ReadingStatus::Valid
        );
        let invalid = tracker.update(vec![(1, Ok(f64::NAN))], Duration::from_secs(1));
        assert_eq!(invalid[0].status, ReadingStatus::Invalid);
        assert_eq!(invalid[0].moisture_percent, None);
        let stale = tracker.update(vec![], Duration::from_secs(2));
        assert_eq!(stale[0].status, ReadingStatus::Stale);
        assert_eq!(stale[0].raw_hz, Some(10.0));
        assert_eq!(stale[0].age_ms, Some(2000));
    }
    #[test]
    fn missing_calibration_reports_uncalibrated_without_percentage() {
        let cfg = Config {
            device_id: "x".into(),
            name: "X".into(),
            backend: Backend::Simulated,
            sample_interval_ms: 1000,
            stale_after_ms: 2000,
            measurement_window_ms: 1000,
            gpio_chip: "/dev/gpiochip0".into(),
            hardware_confirmed: false,
            panel_enabled: false,
            pump: None,
            sensors: vec![SensorConfig {
                channel: 1,
                name: "Soil".into(),
                calibration: None,
                simulated_hz: 10.0,
            }],
            mqtt: None,
        };
        let mut source = SimulatedSource::new(&cfg);
        let reading = SensorTracker::new(&cfg)
            .update(source.read(), Duration::ZERO)
            .remove(0);
        assert_eq!(reading.status, ReadingStatus::Uncalibrated);
        assert_eq!(reading.raw_hz, Some(10.0));
        assert_eq!(reading.moisture_percent, None);
    }

    #[test]
    fn no_pulses_stays_distinct_from_invalid_and_recovers() {
        let mut cfg = Config::load("examples/simulated.toml").unwrap();
        cfg.sensors.truncate(1);
        let mut tracker = SensorTracker::new(&cfg);
        tracker.update(vec![(1, Ok(10.0))], Duration::ZERO);
        for seconds in [1, 20] {
            let reading = tracker
                .update(
                    vec![(1, Err(SensorError::NoPulses))],
                    Duration::from_secs(seconds),
                )
                .remove(0);
            assert_eq!(reading.status, ReadingStatus::NoSignal);
            assert_eq!(reading.raw_hz, Some(10.0));
            assert_eq!(reading.moisture_percent, None);
            assert_eq!(
                reading.error.as_deref(),
                Some("no moisture pulses in measurement window")
            );
        }
        let reading = tracker
            .update(vec![(1, Ok(8.0))], Duration::from_secs(21))
            .remove(0);
        assert_eq!(reading.status, ReadingStatus::Uncalibrated);
        assert_eq!(reading.raw_hz, Some(8.0));
    }
}
