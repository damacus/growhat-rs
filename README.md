# growhat

An independent, unofficial Rust controller for the Pimoroni Grow HAT Mini.
This project is not affiliated with, endorsed by, supported by or maintained by Pimoroni.

It reads moisture-probe pulse frequencies and publishes raw readings,
calibrated moisture and sensor health to Home Assistant over MQTT.
Optional display and button support provides local readings and remote test messages.
Manual pump commands are available; automatic watering remains disabled.

The controller is tested and continuously exercised on our local Pi Zero W.
Quick local checks use simulation; GitHub CI runs software tests on Ubuntu
and cross-builds the ARMv6 executable.

**[Read the documentation](https://damacus.github.io/growhat-rs/)**,
or browse the [Markdown guides](docs/index.md) on GitHub.

## Why Rust

The Pi Zero W has one ARMv6 core and limited memory. Rust lets us build one
small executable on a development host and copy it to the Pi, without installing
a Rust compiler or controller-specific Python packages there.
It also makes the deployed version explicit and easy to replace as one file.

Early measurements showed lower CPU and memory use than the previous Python
setup. The workloads had different features, so they are not a like-for-like
language benchmark. See the [measurements and methodology](docs/python-rust-baseline.md).

## Quick local checks

```sh
cargo test --locked
cargo run --locked -- --config examples/simulated.toml check-config
cargo run --locked -- --config examples/simulated.toml diagnose --samples 3
```

Diagnostics emit one JSON object per sensor per sample to stdout.
Logs go to stderr. `raw_hz` is pulse frequency, not an ADC reading.
Without measured calibration, moisture is `null` and status is `uncalibrated`.

```json
{"channel":1,"raw_hz":10.0,"moisture_percent":null,"status":"uncalibrated","age_ms":0,"error":null}
```

`diagnose` does not connect to MQTT. `check-config` validates without opening GPIO
or contacting a broker. See [testing](docs/testing.md) for the full checks.

## Build here, test on the Pi

You need Rust/rustup, Python 3.11+, [uv](https://docs.astral.sh/uv/), SSH and SCP.

```sh
python3 scripts/dev.py setup
python3 scripts/dev.py smoke --host pi@YOUR_PI
```

The smoke command cross-builds for ARMv6/glibc 2.28, copies changed files,
and runs three simulated samples on the Pi under a timeout.
Verify the Pi's SSH host key before the first run.
It does not install or restart a service and can run alongside the hardware controller.

Build an optimised executable with `python3 scripts/dev.py build --release`.
ARMv7 and AArch64 executables cannot run on the original Pi Zero W.
See [development](docs/development.md) for build tools, SSH keys and hardware diagnostics.

## Configure real sensors

Copy [examples/hardware.toml](examples/hardware.toml) to ignored `config.local.toml`.
Keep one `[[sensors]]` entry per connected probe, using channels 1–3.
Verify the board, GPIO chip and wiring using the [hardware notes](docs/hardware.md),
then set `hardware_confirmed = true`.

Build on the development host and run diagnostics on the Pi.
Stop any controller owning the sensor GPIO before a separate hardware diagnostic,
and restore it afterwards. See [sensor configuration](docs/sensors.md) for commands.

The defaults sample every five seconds with a one-second measurement window.
Each frequency reading needs at least two pulses within that window.
Calibration uses measured dry and wet frequencies for each probe;
without it, raw Hz remains available. These are relative moisture percentages.

## MQTT and Home Assistant

Edit the existing `[mqtt]` table in your hardware configuration for your broker.
Use a unique, stable `device_id` for each controller and a separate ID for simulation.
Store the password in a private credential file referenced by `password_file`.
Relative credential paths resolve beside the TOML file.

The controller uses MQTT 3.1.1 over TCP on a trusted network; TLS is not implemented.
Home Assistant discovery creates frequency and diagnostic status entities for
each configured channel, plus moisture percentage when calibration is present.
Unhealthy readings make numeric entities unavailable.

See [MQTT and Home Assistant](docs/mqtt.md) for broker configuration,
discovery, availability and reconnect behaviour.

## More documentation

- [Manual pump commands](docs/pumps.md) — CLI, MQTT, Home Assistant and limits.
- [Display and buttons](docs/display.md) — local readings and remote controls.
- [Service installation](docs/installation.md) — commissioning and systemd.
- [Release updates](docs/releases.md) — release binaries and automatic updates.
- [Watering feedback](docs/watering-feedback.md) — read-only observations.
- [Validation records](docs/validation.md) — dated checks and commissioning evidence.

Buzzer, light sensor and automatic watering are deferred.
