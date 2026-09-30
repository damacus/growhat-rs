# Manual pump commands

The Rust controller can run an explicitly requested PWM pulse from its CLI or MQTT. Automatic watering is not enabled. Pump control is absent unless a confirmed hardware config has an enabled `[pump]` section and a list of allowed channels. The packaged service creates `/var/lib/growhat` for its persistent request ledger. The example hardware config shows the fields; the defaults allow at most 5 seconds per pulse and 30 seconds total across pumps in any 24-hour window. These are configuration limits, not a measured water-volume limit. The driver uses 100Hz software PWM through the GPIO character device, so its speed response still needs a physical check; the earlier Python diagnostic used 10kHz PWM.

## CLI requests

For a local program, run `/opt/growhat/growhat --config /etc/growhat/config.toml pump --request-id unique-id --channel 1 --duty-percent 50 --duration-ms 1000` as the `growhat` service user. Request IDs contain 1–64 ASCII letters, digits, underscores or hyphens. Duty is 1–90%, following Pimoroni's 90% maximum. A repeated ID is reported without another pulse. The duration is reserved in the ledger before GPIO is energised, so an interrupted or failed pulse still consumes its budget. Concurrent CLI and MQTT pulses are rejected by a shared lock.

## MQTT requests

For MQTT, set `mqtt_token_file` to a separate, private file containing at least 32 characters. Publish non-retained JSON to `<topic_prefix>/<device_id>/pump/set`, for example `{"request_id":"unique-id","channel":1,"duty_percent":50,"duration_ms":1000,"token":"<secret>"}`. The non-retained acknowledgement at `<topic_prefix>/<device_id>/pump/ack` reports `completed`, `duplicate` or `rejected`. Retained commands and commands without the token are rejected. Keep the token outside version control and restrict broker publish access to this topic; MQTT traffic is not encrypted by this application.

The optional development client [scripts/pump.py](https://github.com/damacus/growhat-rs/blob/main/scripts/pump.py) reads the local ignored `.local/pump-mqtt.token` and waits for the acknowledgement. Run it with `--request-id`, `--channel`, `--duty-percent` and `--duration-ms`; reuse the same request ID if an acknowledgement is lost. The token file on the Pi is `/etc/growhat/pump-mqtt.token` and is readable only by `growhat`.

## Home Assistant

For a Home Assistant pump control, [deploy/home-assistant/growhat.yaml](https://github.com/damacus/growhat-rs/blob/main/deploy/home-assistant/growhat.yaml) defines a script that sends one non-retained 50% pulse for one second. It reads the authenticated MQTT payload from a `!secret` key; [deploy/home-assistant/secrets.example.yaml](https://github.com/damacus/growhat-rs/blob/main/deploy/home-assistant/secrets.example.yaml) shows the key format. Install the package as `/config/packages/growhat.yaml` and put the substituted key in `/config/secrets.yaml`, never in MQTT discovery. Check Home Assistant configuration before reloading scripts, and add `script.growhat_pump_1s` to a dashboard. The controller still enforces its per-pulse and rolling MQTT limits. Test the script only while someone watches a pump with its inlet in water.

## Supervised debug mode

For supervised manual debugging only, `pump.debug_mode = true` lets the local CLI bypass the configured per-pulse and rolling 24-hour limits. The CLI still requires an explicit duration and has a five-minute hard cutoff to stop a mistyped request from running indefinitely. MQTT and Home Assistant retain both configured limits. Leave debug mode disabled for normal use; the default is `false`.

## Watering feedback

To measure how quickly soil moisture responds to a manual pump pulse, use the read-only [watering feedback monitor](watering-feedback.md). It records raw frequency, validity and pump acknowledgements without assuming a calibrated percentage or starting a pump. Automatic watering remains disabled.
