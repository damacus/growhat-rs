# Service installation

[deploy/growhat.service](https://github.com/damacus/growhat-rs/blob/main/deploy/growhat.service) expects the binary at `/opt/growhat/growhat`, configuration at `/etc/growhat/config.toml`, and a `growhat` system user with membership of the Pi's `gpio` and `spi` groups. Create that user, install the release binary/configuration and copy the unit into `/etc/systemd/system/`. The unit permits `/dev/gpiochip0` and `/dev/spidev0.0`; adjust its `DeviceAllow` if the verified GPIO chip differs. Keep credentials readable only by the service user.

Then validate the unit with `systemd-analyze verify`, reload systemd and enable/start `growhat.service`. These are commissioning steps; the development smoke command performs none of them. Check logs with `journalctl -u growhat.service`. Preserve the last working executable before upgrading; stop the service and restore it if a release fails.
