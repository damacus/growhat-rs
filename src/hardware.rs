#[cfg(target_os = "linux")]
use crate::config::Backend;
use crate::{
    config::Config,
    sensor::{SensorError, SensorSource},
};
use anyhow::{Result, bail};

#[cfg(any(target_os = "linux", test))]
#[derive(Default)]
struct PulseWindow {
    first_ns: Option<u64>,
    last_ns: Option<u64>,
    count: u64,
}

#[cfg(any(target_os = "linux", test))]
impl PulseWindow {
    fn push(&mut self, timestamp_ns: u64, start_ns: u64, end_ns: u64) {
        if timestamp_ns < start_ns || timestamp_ns >= end_ns {
            return;
        }
        self.first_ns.get_or_insert(timestamp_ns);
        self.last_ns = Some(timestamp_ns);
        self.count += 1;
    }

    fn frequency(&self, overflow: bool) -> std::result::Result<f64, SensorError> {
        if overflow {
            return Err(SensorError::Other(
                "GPIO edge sequence gap; possible overflow".into(),
            ));
        }
        if self.count == 0 {
            return Err(SensorError::NoPulses);
        }
        if self.count < 2 {
            return Err(SensorError::InsufficientPulses);
        }
        let span_ns = self.last_ns.unwrap() - self.first_ns.unwrap();
        if span_ns == 0 {
            return Err(SensorError::Other("duplicate pulse timestamps".into()));
        }
        Ok((self.count - 1) as f64 * 1_000_000_000.0 / span_ns as f64)
    }
}

#[cfg(any(target_os = "linux", test))]
fn sequence_gap(previous: &mut Option<u32>, current: u32, in_window: bool) -> bool {
    if !in_window {
        return false;
    }
    // ABI v1 has no sequence numbers. Hardware uses ABI v2, so zero is invalid.
    let gap = current == 0 || previous.is_some_and(|seq| current != seq.wrapping_add(1));
    *previous = Some(current);
    gap
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use gpiocdev::{
        Request,
        line::{EdgeDetection, EventClock},
    };
    use std::time::{Duration, Instant};

    // BCM GPIO offsets, as used by the official Pimoroni grow/moisture.py.
    const PINS: [u32; 3] = [23, 8, 25];

    pub struct HardwareSource {
        request: Request,
        channels: Vec<u8>,
        window: Duration,
    }

    impl HardwareSource {
        pub fn new(config: &Config) -> Result<Self> {
            config.validate()?;
            if !matches!(config.backend, Backend::Hardware) || !config.hardware_confirmed {
                bail!("hardware backend and explicit hardware confirmation required");
            }
            let channels: Vec<u8> = config.sensors.iter().map(|s| s.channel).collect();
            let pins: Vec<u32> = channels.iter().map(|c| PINS[(c - 1) as usize]).collect();
            let request = Request::builder()
                .on_chip(config.gpio_chip.clone())
                .with_lines(&pins)
                .as_input()
                .with_edge_detection(EdgeDetection::RisingEdge)
                .with_event_clock(EventClock::Monotonic)
                .with_consumer("growhat-moisture")
                .request()?;
            Ok(Self {
                request,
                channels,
                window: Duration::from_millis(config.measurement_window_ms),
            })
        }
    }

    impl SensorSource for HardwareSource {
        fn read(&mut self) -> Vec<(u8, std::result::Result<f64, SensorError>)> {
            // Drain events accumulated while the caller slept. The kernel buffer is small,
            // but cap the drain so a faulty high-rate input cannot block indefinitely.
            const MAX_DRAIN_EVENTS: usize = 4096;
            const MAX_DRAIN_TIME: Duration = Duration::from_millis(100);
            let drain_deadline = Instant::now() + MAX_DRAIN_TIME;
            let mut drain_complete = false;
            for _ in 0..MAX_DRAIN_EVENTS {
                if Instant::now() >= drain_deadline {
                    return self.error_samples("GPIO old-event drain exceeded 100 ms");
                }
                match self.request.has_edge_event() {
                    Ok(false) => {
                        drain_complete = true;
                        break;
                    }
                    Ok(true) => match self.request.read_edge_event() {
                        Ok(_) => {}
                        Err(error) => {
                            return self
                                .error_samples(format!("GPIO old-event drain failed: {error}"));
                        }
                    },
                    Err(error) => {
                        return self.error_samples(format!("GPIO old-event drain failed: {error}"));
                    }
                }
            }
            if !drain_complete {
                return self.error_samples("GPIO old-event drain exceeded 4096 events");
            }
            let mut pulses = [
                PulseWindow::default(),
                PulseWindow::default(),
                PulseWindow::default(),
            ];
            let mut overflow = false;
            let mut in_window_seq = None;
            let start_ns = match monotonic_ns() {
                Ok(ns) => ns,
                Err(error) => {
                    return self
                        .channels
                        .iter()
                        .map(|c| (*c, Err(SensorError::Other(error.clone()))))
                        .collect();
                }
            };
            let end_ns =
                start_ns.saturating_add(self.window.as_nanos().min(u64::MAX as u128) as u64);
            let deadline = Instant::now() + self.window;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match self.request.wait_edge_event(remaining) {
                    Ok(true) => match self.request.read_edge_event() {
                        Ok(event) => {
                            let in_window =
                                event.timestamp_ns >= start_ns && event.timestamp_ns < end_ns;
                            overflow |= sequence_gap(&mut in_window_seq, event.seqno, in_window);
                            if let Some(index) = PINS.iter().position(|pin| *pin == event.offset) {
                                pulses[index].push(event.timestamp_ns, start_ns, end_ns);
                            }
                        }
                        Err(error) => {
                            return self
                                .channels
                                .iter()
                                .map(|c| {
                                    (
                                        *c,
                                        Err(SensorError::Other(format!(
                                            "GPIO edge read failed: {error}"
                                        ))),
                                    )
                                })
                                .collect();
                        }
                    },
                    Ok(false) => break,
                    Err(error) => {
                        return self
                            .channels
                            .iter()
                            .map(|c| {
                                (
                                    *c,
                                    Err(SensorError::Other(format!(
                                        "GPIO edge read failed: {error}"
                                    ))),
                                )
                            })
                            .collect();
                    }
                }
            }
            self.channels
                .iter()
                .map(|channel| {
                    (
                        *channel,
                        pulses[(*channel - 1) as usize].frequency(overflow),
                    )
                })
                .collect()
        }
    }

    impl HardwareSource {
        fn error_samples(
            &self,
            error: impl Into<String>,
        ) -> Vec<(u8, std::result::Result<f64, SensorError>)> {
            let error = SensorError::Other(error.into());
            self.channels
                .iter()
                .map(|channel| (*channel, Err(error.clone())))
                .collect()
        }
    }

    fn monotonic_ns() -> std::result::Result<u64, String> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `time` is a valid writable timespec and CLOCK_MONOTONIC is supported on Linux.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
            return Err(format!(
                "CLOCK_MONOTONIC failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        u64::try_from(time.tv_sec)
            .ok()
            .and_then(|sec| sec.checked_mul(1_000_000_000))
            .and_then(|base| {
                u64::try_from(time.tv_nsec)
                    .ok()
                    .and_then(|nanos| base.checked_add(nanos))
            })
            .ok_or_else(|| "invalid CLOCK_MONOTONIC timestamp".into())
    }
}

#[cfg(target_os = "linux")]
pub use linux::HardwareSource;

#[cfg(not(target_os = "linux"))]
pub struct HardwareSource;

#[cfg(not(target_os = "linux"))]
impl HardwareSource {
    pub fn new(_config: &Config) -> Result<Self> {
        bail!("GPIO hardware is supported only on Linux")
    }
}

#[cfg(not(target_os = "linux"))]
impl SensorSource for HardwareSource {
    fn read(&mut self) -> Vec<(u8, std::result::Result<f64, SensorError>)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignores_old_and_future_edges_and_uses_event_span() {
        let mut p = PulseWindow::default();
        for ns in [1, 100, 300_000_100, 600_000_100, 1_000_000_100] {
            p.push(ns, 100, 1_000_000_100);
        }
        assert_eq!(p.count, 3);
        assert!((p.frequency(false).unwrap() - (2.0 / 0.6)).abs() < 0.00001);
    }
    #[test]
    fn missing_and_single_low_frequency_pulse_are_invalid() {
        let mut p = PulseWindow::default();
        assert!(p.frequency(false).is_err());
        p.push(500_000_000, 0, 1_000_000_000);
        assert!(p.frequency(false).is_err());
    }
    #[test]
    fn idle_old_head_to_first_current_jump_is_not_a_gap() {
        // The kernel may drop newer idle events while retaining its oldest FIFO entries.
        // Neither old sequence 2 nor the first current sequence 100 is part of a pair.
        let mut in_window_seq = None;
        assert!(!sequence_gap(&mut in_window_seq, 2, false));
        assert!(!sequence_gap(&mut in_window_seq, 100, true));
        assert!(!sequence_gap(&mut in_window_seq, 101, true));
        // The next call creates an independent measurement window.
        let mut next_window_seq = None;
        assert!(!sequence_gap(&mut next_window_seq, 500, true));
    }
    #[test]
    fn missing_edge_between_in_window_events_invalidates_frequency() {
        let mut previous = None;
        assert!(!sequence_gap(&mut previous, 100, true));
        assert!(sequence_gap(&mut previous, 102, true));
        assert!(!sequence_gap(&mut previous, 103, true));
        let mut p = PulseWindow::default();
        p.push(100, 0, 1000);
        p.push(200, 0, 1000);
        assert!(p.frequency(true).is_err());
    }
}
