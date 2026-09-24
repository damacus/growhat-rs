# Validation evidence

Recorded during implementation on 22 September 2026. This separates software checks from physical commissioning.

## Display/button follow-up

The user subsequently requested restoration of buttons and MQTT test messages on the LCD. The optional `panel_enabled` implementation is complete locally: 28 Rust tests pass, Clippy with warnings denied passes, and the ARMv6 release cross-build succeeds. The panel settings and orientation were compared with the archived working ST7735 Python driver. Tests cover initial button snapshots, debouncing, message layout/glyphs, command rejection, exact legacy Home Assistant discovery topics and reconnect snapshot coalescing. The local broker suite also checks rejection of display commands while the panel is disabled.

The panel build is deployed on `.216`. The guarded installer saved the sensor-only binary, config and unit first. `growhat.service` is active with zero restarts, and both old Python units remain disabled. The user-requested `RUST DISPLAY TEST` message was accepted by MQTT and the device logged `DISPLAY rendered text=RUST DISPLAY TEST` after an SPI frame write. The broker also returned released/zero-count snapshots for A, B, X and Y. A person still needs to confirm the physical LCD appearance and press a physical button to verify its visible/pressed response.

## Passed

- Native Rust unit tests: 10 sensor/config/GPIO accumulator cases and 3 MQTT payload/freshness/queue cases.
- Native CLI integration tests: 4 cases for bounded NDJSON diagnostics, unconfirmed hardware rejection, sample-count validation and version without configuration.
- Formatting and Clippy with warnings denied.
- ARMv6 Linux cross-compilation of a minimal executable and the full controller using Rust 1.98.1, cargo-zigbuild 0.20.1, Zig 0.13.0 and glibc baseline 2.28.
- Local Mosquitto integration: discovery and calibrated telemetry; Home Assistant birth with stable entity identity; broker restart; clean offline publication; process-death Last Will; calibration removal preserving raw readings; a 1.5-second delayed CONNACK while sampling every 200ms.
- Local simulated diagnostic emits expected raw frequency and uncalibrated status.
- A warm `pi-dev` cross-build took 1.77 seconds wall-clock on the Mac (Cargo reported 0.13 seconds).
- The default `pi-dev` build keeps incremental compilation and omits debug information: 3.5MiB on disk, down from the normal debug binary's 45MiB. This reduces transfer size without enabling release optimisation. Native debug builds are unchanged.

- After the Pi moved to `192.168.1.216`, the full ARM release and incremental binaries ran on the actual Zero. The working Python driver and GPIO chip mapping confirmed channel 1 on BCM23 before Rust requested it.
- Existing Python services were baselined and archived before being disabled. The corrected Rust service passed three repeated hardware diagnostics, systemd validation and live MQTT checks. It is enabled and running; both Python services are disabled and retained for rollback.
- Home Assistant visibly displayed the new raw sensor at 10.1613Hz and its status as `uncalibrated`. Live broker readings arrived every five seconds with no errors, and availability was online.
- The 30-second Rust release observation measured 0.13% CPU and 3.08MiB RSS. See the [Python comparison and rollback record](python-rust-baseline.md) for methodology and workload differences.
- Warm `python3 scripts/dev.py smoke --samples 1` completed in 3.24 seconds: build 1.53s, transfer/checks 1.56s, simulated diagnostic 0.14s (rounding accounts for the sum). Binary and config were unchanged. A changed binary also passed transfer and execution. These are simulated feedback timings, not a two-second hardware measurement window.
- The initial unshared-SSH changed-binary cycle took 27.31 seconds, dominated by 23.55 seconds in transfer/setup. Reusing one SSH connection reduces this overhead. The 3.24-second warm check excludes copying a changed binary, so it is not a like-for-like speedup ratio.
- The temporary SSH key bootstrapped on `.216` expires after seven days and disables forwarding and PTY access. It is ignored by Git and documented under `.local/ssh/` for revocation.
- The panel follow-up has 28 native tests and Clippy with warnings denied; the ARMv6 release cross-build passed. The local Mosquitto suite passes all eight scenarios, including disabled-panel rejection.

## Still to commission

- Deliberately change the physical probe condition and confirm the corresponding raw-frequency response. Steady readings matching Python are not proof of that response.
- Measure calibration endpoints if a percentage is wanted. Current output correctly preserves raw Hz without inventing a percentage.
- Recheck channel 2 after reseating or swapping its probe. The latest bounded diagnostic found no pulses on that channel, while channels 1 and 3 were healthy.
- The exact PCB revision is not available from the device tree; channel wiring follows the manufacturer's driver and the all-channel diagnostic succeeded.

## Three-channel configuration update (23 September 2026)

The live TOML now configures channels 1–3 and `mosquitto.ironstone.casa`. The first bounded Pi diagnostic read all channels three times: channel 1 was 10.49–10.56Hz and channels 2 and 3 were approximately 6.33Hz. The previous service config was saved as a timestamped rollback copy. DNS now resolves on the Pi, the service connects to MQTT, and retained Rust discovery exists for all three channels. The Home Assistant screenshot shows channel 1 and 3 raw readings and channel 2 status `invalid`. The old Python frequency entities remain unavailable because their state topic is no longer published.

A later three-sample hardware diagnostic, with the service stopped and restored afterwards, read channel 1 at 10.246–10.251Hz and channel 3 at 3.923–3.925Hz. Channel 2 had no pulses in every window. GPIO8 was configured as an input, and the service was active after the diagnostic. This narrows the next check to the probe/connector and channel 2 input; swapping probes 2 and 3 will distinguish them.

After swapping the probes at ports 2 and 3, a further three-sample diagnostic read channel 2 at 4.098–4.127Hz, while channel 3 had no pulses each time. Live MQTT showed the same result. The fault followed the probe or cable; the HAT's channel 2 input and Rust channel mapping work. The service was restored and remained active with zero automatic restarts. The person at the HAT reported no visible display update after the swap; the current screen contents still need confirmation before attributing that to rendering.

Pump 1's existing Grow HAT output was tested separately from the Rust service with `scripts/pump1_test.py`, using the manufacturer's 10kHz software PWM mapping. The attended test used 10% for one second, then 50% for one second, with a five-second external timeout. Both pulses were observed to run; their speeds looked similar during the brief test. The script returned the output low and GPIO17 was then restored to input. The Rust service stayed active and the Pi reported no undervoltage. This verifies basic actuation, not flow rate, electrical margin, or unattended watering behaviour.

The Rust service now has manual-only pump 1 commands through its CLI and MQTT, using 100Hz software PWM. Native tests cover request validation, duplicate IDs, persistent duration budget and corrupt-state refusal. The local Mosquitto suite confirmed disabled and retained requests are rejected. The ARMv6 release built and was installed with prior binary/config/unit copies under `/opt/growhat/pump-rollbacks/20260923-202143`. Live `check-config` passed, the service connected to MQTT without restarting, all three sensor state topics continued publishing, GPIO17 stayed input/low, a zero-duration CLI request was rejected, and an authenticated zero-duration MQTT request was rejected before GPIO. No Rust pump pulse has yet been physically observed, so the new PWM frequency's motor response remains unverified.

Hardware diagnostics need exclusive GPIO access: stop the running controller first and restore it afterwards. The default simulated smoke command can run alongside the service.

## Home Assistant sensor presentation (23 September 2026)

Live MQTT reported channel 1 at about 9.6Hz and channel 2 at about 3.46Hz, both uncalibrated; channel 3 reported no pulses and `invalid`. These are current Pi readings, not cached values from the Python publisher. The four retained Python discovery topics for the three old `Moisture N frequency` entities and `Illuminance` were backed up to ignored `.local/legacy-python-discovery-20260923.json` and cleared. The three previous Rust `raw` discovery configurations were backed up to `.local/legacy-rust-raw-discovery-20260923.json` before the controller replaced them with frequency entities. The old raw entity history remains under its previous Home Assistant entity IDs.

The ARMv6 release was deployed with a rollback binary at `/opt/growhat/growhat.before-ha-frequency`. Live broker inspection found three retained frequency discovery configurations and none of the seven obsolete configurations. The Home Assistant device page visibly showed Anthurium frequency at 9.63Hz and Sensor 2 frequency at 3.46Hz in its main Sensors section; Sensor 3 frequency was unavailable. No moisture percentage is published until probe-specific dry and wet calibration values are measured. The service remained active and pump GPIO17 was low after deployment.

## Remote controls (24 September 2026)

The controller now discovers four Home Assistant `Press A/B/X/Y on HAT` controls. A virtual press updates the corresponding count and shows `REMOTE <letter>` on the LCD without changing pump output. MQTT pump commands now enforce the rolling 24-hour budget even when local CLI debug mode is enabled. Native Rust tests (32 cases), formatting, Clippy and the ARMv6 release build pass.

The installed Pi binary matches the local release SHA-256. The service connected to MQTT after restart, with the prior executable at `/opt/growhat/growhat.before-remote-controls`. Live broker discovery for A names the expected command topic. A non-retained MQTT press was acknowledged and published count 1. The Home Assistant device page displayed all four controls; pressing A there logged `BUTTON virtual press A` and `DISPLAY rendered text=REMOTE A` on the Pi. GPIO17 remained output-low. No pump pulse was requested during this validation.

The Home Assistant pump script and secret format are in `deploy/home-assistant/`. After explicit approval, a temporary pod running as the configuration volume owner installed `/config/packages/growhat.yaml` and a private `/config/secrets.yaml` key. The package SHA-256 matched the repository template, and the Home Assistant pod could read both files. `python -m homeassistant --script check_config --config /config` exited successfully; it also repeated two pre-existing, unrelated missing-device automation warnings. The script was reloaded through Home Assistant's Tools page and appeared in the Scripts list as `Grow HAT pump 1 - 50% for 1 second`. Its secret-backed payload rendered valid JSON with channel 1, 50% duty and a 1000ms duration; the token was not displayed. The temporary writer pod was removed. The script has **not** been run, so physical pump response from Home Assistant remains unverified. The Pi service remained active with GPIO17 output-low.

## Display freshness follow-up (24 September 2026)

The user reported that the HAT screen appeared to stop updating. Live MQTT samples continued every five seconds, with channel 1 moving only from 9.8478 to 9.8520Hz and channel 2 from 3.4146 to 3.4158Hz over three samples; the screen rounded readings to one decimal place, so it could look static. A temporary `DISPLAY CHECK 07:02` message was accepted and the Pi logged its SPI frame write. The user then reported that the button counts matched Home Assistant when the normal screen returned, before a service restart.

The normal screen now shows a six-digit sample counter. Native tests, Clippy and the ARMv6 release build passed. The new executable matches the Pi's installed SHA-256; its predecessor is `/opt/growhat/growhat.before-display-counter`. After restart, Pi logs showed consecutive screen draws for samples 24–29 at five-second intervals, and GPIO17 remained output-low. Physical confirmation that the counter advances on the LCD is still needed; SPI write logs alone cannot prove what the glass displays.

## Requested pump 1 pulse (24 September 2026)

The user requested a single 50% pulse lasting 10 seconds. The installed Home Assistant script remained at one second, so this test used the authenticated MQTT development client rather than silently changing the HA control. The Pi's prior rolling ledger already contained 35 seconds of local debug pulses. For this test only, the configured per-pulse limit was raised from 5 to 10 seconds and the rolling cap from 30 to 45 seconds, with an automatic two-minute rollback timer. The MQTT acknowledgement reported `completed` for request `manual-20260924-50pct-10s`; the ledger contains exactly one 10,000ms entry for it. The original limits were then restored, the rollback timer stopped, the service remained active, and GPIO17 was output-low. Physical motor and water-flow confirmation is pending the observer's report. With 45 seconds currently in the rolling ledger, subsequent MQTT pump requests will be rejected until enough entries age out of the 24-hour window.

For subsequent manual experimentation, the local CLI's `debug_mode = true` now bypasses both configured pulse and rolling limits, while requiring an explicit duration no greater than five minutes. MQTT continues to enforce both configured limits. A native unit test checks the debug-only bypass and hard cutoff; 33 tests, Clippy and the ARMv6 release build passed. The installed Pi binary matched the local SHA-256 and passed `check-config`; its predecessor is `/opt/growhat/growhat.before-debug-bypass`. During verification, another local `growhat pump` process was already running a user-initiated 10-second pulse. GPIO17 was subsequently low, no pump process remained, and `growhat.service` remained active. The deployment itself did not request a pulse.

## Unplugged sensor and watering feedback (24 September 2026)

With sensor 2 unplugged, live MQTT marked it `stale` and preserved its last raw reading; sensor 1 remained near 9.95Hz and sensor 3 was invalid. The Pi reported `throttled=0x0`, kept its service active, and had only the Rust process using `/dev/spidev0.0`. The normal renderer contains no red colour or invalid-status red screen. Around the reported artifact, 43 frames were logged in a minute versus the usual 12, with no virtual button presses logged; physical button transitions or electrical noise may explain the extra redraws, but neither is proven. `SCREEN CHECK 08:01` was accepted and the Pi logged both its frame write and the normal sensor frame after its 30-second expiry. Whether either frame looked correct on the physical LCD is still unverified.

A read-only MQTT commissioning monitor was added as `scripts/water_feedback.py`. A seven-second live trial printed healthy channel 1 raw frequency without requesting a pump pulse. It can record a timed pulse/settle trace and, once a wet-enough target is measured, report repeated healthy samples at or below that target. Automatic pump control remains disabled.

On 24 September, with probe 3 reported unplugged, the live controller continued sampling and did not restart. Before the status change, MQTT reported channel 3 as `invalid` with no raw frequency. The updated controller reports `no_signal` for zero pulses, while the LCD renders `M3: NO SIGNAL`; one pulse and GPIO read faults remain `invalid`. Fresh MQTT state confirmed `no_signal` on channel 3, and channels 1 and 2 continued publishing frequencies. The service had zero automatic restarts and the installed ARMv6 binary matched the local SHA-256 after the atomic deployment. The LCD's physical appearance still needs an observer check. Pulse absence cannot distinguish an unplugged probe from a faulty or non-oscillating probe.
