# Growhat

Growhat is an independent, unofficial Rust controller for the Pimoroni Grow HAT Mini.
It is not affiliated with, endorsed by, supported by or maintained by Pimoroni.

The controller reads moisture-probe pulse frequencies and publishes raw readings,
calibrated moisture and health to Home Assistant over MQTT. Optional display and
button support provides local readings and remote controls. Manual pump commands
are available; automatic watering remains disabled.

The controller is tested and continuously exercised on our local Pi Zero W.
Software CI runs on Ubuntu and cross-builds for ARMv6; there is no Pi CI runner.

## Getting started

1. Check the [hardware and wiring](hardware.md).
2. [Build and run a simulated Pi diagnostic](development.md).
3. [Configure your real sensors](sensors.md).
4. [Connect MQTT and Home Assistant](mqtt.md).
5. [Install the service](installation.md) after commissioning.

## Operation

- [Manual pump commands](pumps.md)
- [Display and buttons](display.md)
- [Watering feedback monitor](watering-feedback.md)
- [Release updates](releases.md)

## Development

- [Development workflow](development.md)
- [Testing and documentation checks](testing.md)
- [Source code](https://github.com/damacus/growhat-rs)

## Historical records

These dated records describe previous checks and observations, rather than current live state.

- [Validation evidence](validation.md)
- [Python baseline and Rust handover](python-rust-baseline.md)
- [Original implementation plan](implementation-plan.md)
