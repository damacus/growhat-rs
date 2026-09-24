# Python baseline and Rust handover

Measured on 22 September 2026 on `pi@192.168.1.216`, the Pi Zero W Rev 1.1. Python was measured and backed up before either existing service was disabled.

## Baseline

The existing `grow-hat-input-test.service` ran `/home/pi/grow-display-test/input_test.py`. It counted moisture edges on BCM23/8/25, refreshed the display every 250ms, read the light sensor every 500ms and handled four buttons. It rewrote `input-status.json` each display frame. `grow-dashboard-mqtt.service` read that file every 100ms and published to the existing MQTT broker every five seconds or on button changes.

Channel 1 reported 9.8–10.4Hz during the baseline. Channels 2 and 3 reported zero. There were no reported sensor errors; the oldest observed state file was 0.384 seconds old. Both services were enabled, running and had zero systemd restarts. Neither script initialised pumps or the buzzer.

Each resource measurement lasted 30 seconds, with `/proc` snapshots every two seconds. CPU is a percentage of the Zero's single core. RSS is resident memory; adding process RSS can count shared pages more than once.

| Workload | CPU | Mean RSS | Process storage writes |
|---|---:|---:|---:|
| Python input/display | 35.72% | 26.48 MiB | 288 KiB |
| Python MQTT | 1.90% | 20.82 MiB | 0 |
| Python combined | 37.62% | 47.29 MiB | 288 KiB |
| Rust sensor/MQTT release | 0.13% | 3.08 MiB | 0 |

This is a comparison of the deployed workloads, not an isolated language benchmark: Rust was measured with only the connected moisture channel and MQTT active. Display and button support were added later; the light sensor remains deferred. These short observations are not a long-term stability claim.

## Handover and live checks

The first guarded handover read 10.16Hz, then rejected subsequent windows because it treated a sequence-number jump across discarded idle events as a loss inside the measurement window. The guard restored both Python services. The driver now establishes its sequence baseline at the first event inside each window; missing events inside that window still invalidate the reading. Regression tests cover both cases.

The corrected release passed three successive hardware samples (10.168, 10.163 and 10.161Hz). `growhat.service` was then enabled and started, and both Python services disabled. Rust runs as its own `growhat` user, with GPIO group access. The systemd unit and configuration passed on-device validation. No compiler or development dependency was installed on the Pi.

The live configuration reads channel 1 every five seconds, using a two-second measurement window and a 15-second stale threshold. It has no calibration. The provisional dry/watered values in the old Python directory were preserved but not promoted to calibration endpoints.

The real broker delivered four fresh readings at five-second intervals, 10.165–10.167Hz, with `age_ms: 0`, `error: null` and `status: uncalibrated`. Rust availability was retained as `online`; both old Python availability topics were `offline`. Home Assistant at `hass.ironstone.casa` visibly showed:

- `sensor.grow_anthurium_anthurium_raw`: 10.1613Hz, updated three seconds earlier.
- `sensor.grow_anthurium_anthurium_status`: `uncalibrated`.

The existing HA device identity `grow_pi_zero_w` is retained. Rust uses new raw/status entity IDs. Old Python discovery entries and history are preserved. The button entity topics use their previous IDs and are active again under Rust; illuminance publishing remains deferred. Pumps remain untouched.

On 22 September the display/button follow-up enabled `panel_enabled`, added SPI device access and installed a dedicated SSH key. A guarded installer preserved the then-running sensor-only Rust binary, config and unit under the Pi migration staging directory before restart. The service runs with zero restarts. `.local/baselines/20260922-pi216/panel-live-check.txt` records the startup and `DISPLAY rendered text=RUST DISPLAY TEST` evidence. All four button MQTT snapshots reported released/count 0. Physical screen appearance and a physical press are still awaiting direct human confirmation.

Physical response to deliberately changed probe conditions and measured calibration endpoints remain commissioning checks. Normal small fluctuations do not prove that test.

## Evidence and rollback

Detailed captures are private, ignored files under `.local/baselines/20260922-pi216/`: `python-performance.json`, `python-state.txt`, `python-mqtt.json`, `rust-handover.txt`, `rust-performance.json` and `rust-mqtt.json`. The read-only collector is `scripts/baseline.py`; for example:

```sh
ssh pi@192.168.1.216 'sudo -n python3 - --seconds 30 growhat.service' < scripts/baseline.py
```

`python-rollback.tar.gz` contains the original scripts, vendor code, environment file, plant references, service units and boot configuration. Copies exist in that local evidence directory and on the Pi at `/home/pi/.local/growhat-migration/20260922/python-rollback.tar.gz`. Archive SHA-256:

```text
36090681a167d00548539676c0adae3cee8b891e6aa6ac0e7bc587757d8e34cd
```

The Python files and unit definitions were left in place. To roll back:

```sh
ssh pi@192.168.1.216 'sudo systemctl disable --now growhat.service && sudo systemctl enable --now grow-hat-input-test.service grow-dashboard-mqtt.service'
```

To return to Rust, stop/disable both Python units before enabling Rust; they must not own the same sensor GPIO simultaneously. `/opt/growhat/growhat.previous` holds the first, rejected Rust candidate, **not** a known-good fallback. Use Python for rollback until a later validated Rust upgrade supplies a known-good previous release.

Installed corrected Rust executable SHA-256:

```text
534abf54331b1ce4183fe772c4fc4bf695245bf56c0825f2381da204668775fd
```
