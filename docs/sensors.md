# Configure real sensors

Copy [examples/hardware.toml](https://github.com/damacus/growhat-rs/blob/main/examples/hardware.toml) to ignored `config.local.toml`. The TOML supports channels 1–3; add one `[[sensors]]` entry per connected probe and remove entries for unused channels. `diagnose` checks configured channels without MQTT, so sensor discovery is explicit configuration rather than automatic GPIO scanning. Verify the board, GPIO chip and connected channels using the [hardware notes](hardware.md), then set `hardware_confirmed = true`. Run a diagnostic on the Pi before relying on MQTT entities. A hardware diagnostic needs exclusive GPIO access: stop the running controller before the diagnostic and restore it afterwards. The simulated smoke check can run alongside the service.

Once the binary and configuration are installed on the Pi, run as the service user:

```sh
sudo -u growhat /opt/growhat/growhat --config /etc/growhat/config.toml check-config
sudo -u growhat /opt/growhat/growhat --config /etc/growhat/config.toml diagnose --samples 3
```

For cross-building and staging a development configuration, see [development](development.md).

## Sampling and health

The defaults sample every 5 seconds with a 1-second measurement window and mark an unpublished sensor snapshot stale after 15 seconds. All channels are captured together using Linux GPIO edge events; the process does not busy-poll. A frequency needs at least two edges within the measurement window. Increase the window for unusually low frequencies and keep it no longer than the sample interval.

Set timing fields at the top level, before any `[[sensors]]` or `[mqtt]` table:

```toml
sample_interval_ms = 5000
measurement_window_ms = 1000
stale_after_ms = 15000
```

`stale_after_ms` must be at least the sample interval. Each timing value must be between 1 and 3,600,000 milliseconds.

## Calibration

Each sensor can have measured calibration endpoints:

```toml
[[sensors]]
channel = 1
name = "Basil"
# Add calibration only after measuring dry_hz and wet_hz for this probe/setup.
# [sensors.calibration]
# dry_hz = <measured dry frequency>
# wet_hz = <measured wet frequency>
```

Calibration maps dry to 0% and wet to 100%, clamps beyond those endpoints, and supports either frequency direction. These are relative calibration percentages, not a claim of laboratory volumetric water content. Zero pulses report `no_signal`, one pulse or a GPIO read fault reports `invalid`, and an old unpublished snapshot reports `stale`. The HAT has no separate probe-presence signal, so `no_signal` cannot prove a connector is unplugged. The last raw frequency may remain in diagnostic output with its age and error; old moisture percentages are removed.

Calibration endpoints must be distinct, positive, finite frequencies. Place `[sensors.calibration]` directly after the sensor it belongs to; each probe needs its own measurements. Remove the calibration table to keep publishing raw Hz without a percentage.
