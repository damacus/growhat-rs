# Release updates

GitHub Actions runs formatting, Rust tests, Clippy, MQTT integration checks and an ARMv6/glibc 2.28 cross-build. A `vX.Y.Z` tag matching the version in `Cargo.toml` creates a stable GitHub release containing the Pi executable and `SHA256SUMS`. The workflow publishes no Rust libraries or build dependencies to the device.

## Automatic updates

To enable automatic updates, install [deploy/growhat-update](https://github.com/damacus/growhat-rs/blob/main/deploy/growhat-update) at `/usr/local/sbin/growhat-update` as root-owned mode `0755`, and install [deploy/growhat-update.service](https://github.com/damacus/growhat-rs/blob/main/deploy/growhat-update.service) and [deploy/growhat-update.timer](https://github.com/damacus/growhat-rs/blob/main/deploy/growhat-update.timer) under `/etc/systemd/system/`. Then run `sudo systemctl daemon-reload && sudo systemctl enable --now growhat-update.timer`. The timer checks GitHub's latest stable release about every six hours. The updater verifies the checksum, confirms the candidate version and checks the device configuration, then atomically replaces the executable and restarts `growhat.service`. If the service does not stay healthy, it restores the previous executable. It downloads only the executable and checksum file; no crates or runtime libraries are installed. The updater runs as root so it can replace the service binary. SHA-256 detects a corrupt or mismatched download but does not independently authenticate the publisher.

## Creating a release

To make a release, change the package version in `Cargo.toml` and the matching root package entry in `Cargo.lock`, commit those changes on `main`, and push a matching annotated tag (for example, `v0.2.0`). The release job runs only after all CI and the ARMv6 build pass. The device follows stable releases, rather than arbitrary commits on `main`.

Buzzer, light sensor and automatic watering are deferred. [Manual pump commands](pumps.md) are available.
