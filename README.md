# growhat

An independent, unofficial Rust port for the Pimoroni Grow HAT Mini. This project is not affiliated with, endorsed by or supported by Pimoroni. It reads moisture-probe pulse frequencies and publishes raw readings, calibrated moisture and health to Home Assistant over MQTT. Optional display and button support provides local readings and temporary test messages. Pump 1 accepts bounded manual CLI and authenticated MQTT pulse requests; automatic watering is disabled.

## Why Rust

The first version used Python, but the Pi Zero W has one ARMv6 core and limited memory. Rust lets us compile one small executable on the development Mac and copy it to the Pi without installing a Rust compiler or controller-specific Python packages there. In a short baseline, the Python sensor/display and MQTT processes together used 37.62% CPU and 47.29 MiB mean RSS; an early Rust sensor/MQTT build used 0.13% CPU and 3.08 MiB. Those measurements used different feature sets, so they show the expected direction rather than a like-for-like benchmark. The Rust build also makes the version deployed to the Pi explicit and easy to replace as one file.

## Quick local check

```sh
cargo test --locked
cargo run --locked -- --config examples/simulated.toml diagnose --samples 3
```

Diagnostics emit one JSON object per sensor per sample to stdout. Errors and logs go to stderr. `raw_hz` is pulse frequency, not an ADC reading. Without calibration, `moisture_percent` is `null` and status is `uncalibrated`.

```json
{"channel":1,"raw_hz":10.0,"moisture_percent":null,"status":"uncalibrated","age_ms":0,"error":null}
```

`diagnose` never connects to MQTT. It exits unsuccessfully if any sample is invalid or stale. Exit codes are 0 for success, 1 for runtime or reading failures, and 2 for invalid arguments/configuration. `growhat --help` lists commands; `check-config` validates without opening GPIO or contacting a broker.

## Build here, test on the Pi

Prerequisites: Rust/rustup, Python 3.11+, [uv](https://docs.astral.sh/uv/), SSH and SCP. The tested development host is Apple Silicon macOS. Setup pins Rust 1.98.1, Zig 0.13.0 and cargo-zigbuild 0.20.1; Python tools and compiler caches stay in ignored `.tools/`.

```sh
python3 scripts/dev.py setup
python3 scripts/dev.py smoke
```

The smoke command cross-compiles `arm-unknown-linux-gnueabihf` for glibc 2.28, copies changed files to `pi@192.168.1.216:~/.local/growhat-dev/`, then runs three simulated samples under a remote timeout. It prints build, transfer and diagnostic timings. It uses the existing SSH host trust and key; connect interactively and verify the host key first if it is unknown.

For a dedicated local development key, use `--identity PATH`. The default Pi automatically uses ignored `.local/ssh/growhat_pi_ed25519` when present, with the SSH agent disabled. Creating a local key does not authorise it remotely: install its public key on the Pi through an existing authenticated connection first. Temporary key expiry and revocation details are kept alongside the local key.

No Rust compiler is installed on the Pi. The command uses cached, incremental `pi-dev` builds without debug information to reduce transfer size. Native development builds retain normal debugging information. One SSH connection is reused for the invocation and closed afterwards. The command checks file hashes, stages the executable before replacing it, and saves `growhat.previous`. It does not install or restart a system service. Only one smoke invocation should target this directory at a time.

```sh
# Optimised binary for device CPU/memory/timing checks:
python3 scripts/dev.py build --release
# A confirmed, local hardware config (stop any service owning its GPIO first):
python3 scripts/dev.py smoke --release --hardware --config config.local.toml --samples 3
```

The confirmed target is a Pi Zero W Rev 1.1 (ARMv6), 32-bit Raspbian 13 with glibc 2.41. ARMv7 and AArch64 binaries are incompatible with this device. See [hardware notes](docs/hardware.md) and [validation status](docs/validation.md).

The live Pi runs `growhat.service` with channels 1–3 configured. It resolves `mosquitto.ironstone.casa` and publishes Home Assistant discovery and readings. After swapping probes 2 and 3, channels 1 and 2 produce pulses while channel 3 reports none. The fault followed the probe or cable originally connected to channel 2. The previous Python services were baselined, backed up and disabled. See the [measurements and rollback command](docs/python-rust-baseline.md). A warm one-sample simulated smoke takes 3.24 seconds with unchanged files; it can run alongside the hardware service.

## Configure real sensors

Copy `examples/hardware.toml` to ignored `config.local.toml`. The TOML supports channels 1–3; add one `[[sensors]]` entry per connected probe and remove entries for unused channels. `diagnose` checks configured channels without MQTT, so sensor discovery is explicit configuration rather than automatic GPIO scanning. Verify the board, GPIO chip and connected channels using the hardware notes, then set `hardware_confirmed = true`. Run a diagnostic before relying on MQTT entities.

The defaults sample every 5 seconds with a 1-second measurement window and mark an unpublished sensor snapshot stale after 15 seconds. All channels are captured together using Linux GPIO edge events; the process does not busy-poll. A frequency needs at least two edges within the measurement window. Increase the window for unusually low frequencies and keep it no longer than the sample interval.

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

## MQTT and Home Assistant

Add a broker section to your local configuration:

```toml
[mqtt]
host = "mosquitto.ironstone.casa"
port = 1883
topic_prefix = "growhat"
discovery_prefix = "homeassistant"
username = "growhat"
password_file = "mqtt.credentials"
```

Put only the password in the credential file and restrict its permissions to the service user. Relative credential paths resolve beside the TOML file. Do not pass passwords as command arguments or commit them. Anonymous brokers can omit both credential fields.

```sh
cargo run --locked -- --config config.local.toml check-config
cargo run --locked -- --config config.local.toml run
```

This version uses MQTT 3.1.1 over TCP on a trusted network; TLS is not implemented. Use a broker that accepts this protocol. Set a unique, stable `device_id` for each controller. A simulated controller must use a different ID from real hardware.

Home Assistant discovery creates a visible frequency sensor and diagnostic status entity for every configured channel, plus moisture percentage when calibration is present. Frequency is shown to two decimal places while the MQTT payload keeps the full measured value. The previous diagnostic `raw` discovery identity is removed on startup; its old recorder history stays under the old entity ID. Removing calibration clears its retained moisture discovery entry. Other removed channels/devices require removing their retained discovery topics manually; changing IDs intentionally creates new entities.

The old Python discovery entries `Moisture 1/2/3 frequency` and `Illuminance` are removed from the broker after their retained configuration is backed up. Restoring the Python publisher can restore those entries. Use the Rust frequency entities for current readings. A channel's frequency entity is unavailable while its diagnostic status is `no_signal`, `invalid` or `stale`; the status JSON includes the sensor error.

Discovery is retained. State is not retained, and numeric entities require both device availability and a healthy reading. `expire_after` prevents a stalled sampler appearing current. The status entity exposes `no_signal`, `invalid` or `stale` and JSON attributes include sample age and error. The app republishes discovery after broker reconnect and the default Home Assistant birth message (`homeassistant/status`, or the configured discovery prefix plus `/status`). The Last Will reports offline after connection loss; SIGINT/SIGTERM also publish offline.

An outage retains only the latest sensor snapshot in memory. Reconnection discards the failed MQTT client's pending requests and reevaluates snapshot age before publishing.

## Manual pump commands

The Rust controller can run an explicitly requested PWM pulse from its CLI or MQTT. Automatic watering is not enabled. Pump control is absent unless a confirmed hardware config has an enabled `[pump]` section and a list of allowed channels. The packaged service creates `/var/lib/growhat` for its persistent request ledger. The example hardware config shows the fields; the defaults allow at most 5 seconds per pulse and 30 seconds total across pumps in any 24-hour window. These are configuration limits, not a measured water-volume limit. The driver uses 100Hz software PWM through the GPIO character device, so its speed response still needs a physical check; the earlier Python diagnostic used 10kHz PWM.

For a local program, run `growhat --config /etc/growhat/config.toml pump --request-id unique-id --channel 1 --duty-percent 50 --duration-ms 1000` as the `growhat` service user. Request IDs contain 1–64 ASCII letters, digits, underscores or hyphens. Duty is 1–90%, following Pimoroni's 90% maximum. A repeated ID is reported without another pulse. The duration is reserved in the ledger before GPIO is energised, so an interrupted or failed pulse still consumes its budget. Concurrent CLI and MQTT pulses are rejected by a shared lock.

For MQTT, set `mqtt_token_file` to a separate, private file containing at least 32 characters. Publish non-retained JSON to `<topic_prefix>/<device_id>/pump/set`, for example `{"request_id":"unique-id","channel":1,"duty_percent":50,"duration_ms":1000,"token":"<secret>"}`. The non-retained acknowledgement at `<topic_prefix>/<device_id>/pump/ack` reports `completed`, `duplicate` or `rejected`. Retained commands and commands without the token are rejected. Keep the token outside version control and restrict broker publish access to this topic; MQTT traffic is not encrypted by this application.

The optional development client `scripts/pump.py` reads the local ignored `.local/pump-mqtt.token` and waits for the acknowledgement. Run it with `--request-id`, `--channel`, `--duty-percent` and `--duration-ms`; reuse the same request ID if an acknowledgement is lost. The token file on the Pi is `/etc/growhat/pump-mqtt.token` and is readable only by `growhat`.

For a Home Assistant pump control, `deploy/home-assistant/growhat.yaml` defines a script that sends one non-retained 50% pulse for one second. It reads the authenticated MQTT payload from a `!secret` key; `deploy/home-assistant/secrets.example.yaml` shows the key format. Install the package as `/config/packages/growhat.yaml` and put the substituted key in `/config/secrets.yaml`, never in MQTT discovery. Check Home Assistant configuration before reloading scripts, and add `script.growhat_pump_1s` to a dashboard. The controller still enforces its per-pulse and rolling MQTT limits. Test the script only while someone watches a pump with its inlet in water.

For supervised manual debugging only, `pump.debug_mode = true` lets the local CLI bypass the configured per-pulse and rolling 24-hour limits. The CLI still requires an explicit duration and has a five-minute hard cutoff to stop a mistyped request from running indefinitely. MQTT and Home Assistant retain both configured limits. Leave debug mode disabled for normal use; the default is `false`.

To measure how quickly soil moisture responds to a manual pump pulse, use the read-only [watering feedback monitor](docs/watering-feedback.md). It records raw frequency, validity and pump acknowledgements without assuming a calibrated percentage or starting a pump. Automatic watering remains disabled.

## Tests

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
python3 scripts/dev.py build
.tools/bin/python tests/mqtt_integration.py
```

The last command needs Docker. It starts a disposable Mosquitto container on a localhost-only port, checks discovery, telemetry, Home Assistant birth, broker restart, clean shutdown, process death, missing calibration and a deliberately slow connection handshake, then removes it. It does not use the household broker.

## Display and buttons

Set top-level `panel_enabled = true` in a confirmed hardware configuration. This uses the Grow HAT Mini's existing `/dev/spidev0.0`, DC GPIO9 and backlight GPIO12, plus pull-up button inputs A=5, B=6, X=16 and Y=24. The service needs the `spi` and `gpio` groups and access to both device nodes. The packaged service includes these permissions. Stop other programs using the display or buttons before enabling it.

The normal LCD header shows the Pi's IPv4 address and a rolling sample counter, for example `IP 192.168.1.216 #0042`. The address is selected from the default network route without DNS or an MQTT connection and is checked every five seconds. It shows `IP unavailable` when no address can be selected. Temporary display messages hide the header until they expire.

The normal screen shows moisture readings, button press counts and a four-digit sample counter beside the IP address. The counter advances every sensor cycle even when rounded readings stay the same, making a stalled screen visible. Button states are debounced for 30ms; counts and the sample counter reset when the process restarts. Held-state and press-count entities use the original Python button entity identities, so they return under the same Home Assistant device. No watering action is bound to a button.

Home Assistant also discovers `Press A/B/X/Y on HAT` button controls on the same device. Pressing one sends a non-retained `PRESS` command to `<topic_prefix>/<device_id>/buttons/<letter>/press/set`; the controller increments that button's press count, shows `REMOTE <letter>` briefly and publishes an acknowledgement under `/press/ack`. These virtual presses do not energise a pump or claim that a physical button is held. Limit publish access to these topics if other broker clients should not be able to trigger them.

Send a temporary test message from the Mac:

```sh
.tools/bin/python scripts/display.py "RUST DISPLAY TEST" --seconds 30 --config config.local.toml
```

The controller accepts non-retained JSON on `<topic_prefix>/<device_id>/display/set`:

```json
{"text":"RUST DISPLAY TEST","ttl_seconds":30}
```

Messages accept printable ASCII and newlines, fitting five lines of 24 characters (up to 120 bytes), and last 1–300 seconds (default 30). Letters render in uppercase. The screen returns to moisture readings afterwards. `<topic_prefix>/<device_id>/display/ack` reports whether the message was queued; it is not visual proof of the LCD. The device logs successful frame writes. Messages are rejected if the panel is disabled, the input is invalid or its queue is full. Use the broker's normal access controls; anyone allowed to publish to this command topic can change its temporary screen message.

## Service installation after commissioning

`deploy/growhat.service` expects the binary at `/opt/growhat/growhat`, configuration at `/etc/growhat/config.toml`, and a `growhat` system user with membership of the Pi's `gpio` and `spi` groups. Create that user, install the release binary/configuration and copy the unit into `/etc/systemd/system/`. The unit permits `/dev/gpiochip0` and `/dev/spidev0.0`; adjust its `DeviceAllow` if the verified GPIO chip differs. Keep credentials readable only by the service user.

Then validate the unit with `systemd-analyze verify`, reload systemd and enable/start `growhat.service`. These are commissioning steps; the development smoke command performs none of them. Check logs with `journalctl -u growhat.service`. Preserve the last working executable before upgrading; stop the service and restore it if a release fails.

Buzzer, light sensor and watering are deferred. Ordinary local tests use simulation and do not access hardware.
