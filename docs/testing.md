# Testing

Local checks use simulation. Hardware behaviour is tested on the Pi Zero W, where the controller is continuously exercised during normal operation. This is separate from automated GitHub CI; the repository does not have a Pi CI runner.

## Local software checks

Run [development setup](development.md) first for the cross-builder and Python MQTT dependency. The Docker daemon must be running for the MQTT suite.

```sh
cargo test --locked
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
python3 scripts/dev.py build
.tools/bin/python tests/mqtt_integration.py
```

The last command needs Docker. It starts a disposable Mosquitto container on a localhost-only port, checks discovery, telemetry, Home Assistant birth, broker restart, clean shutdown, process death, missing calibration and a deliberately slow connection handshake, then removes it. It does not use the household broker.
## GitHub CI

The [CI workflow](https://github.com/damacus/growhat-rs/blob/main/.github/workflows/ci.yml) runs formatting, Rust tests, Clippy and the disposable MQTT integration suite on Ubuntu. A separate job cross-builds the ARMv6 release executable for glibc 2.28. A cross-build proves compilation, not on-device execution.

## Bounded Pi checks

```sh
python3 scripts/dev.py smoke --samples 3
```

This builds on the development host, copies changed files and runs three simulated samples on the Pi under a timeout. It can run alongside the hardware service.

For real sensor diagnostics, first stop any service using the same GPIO, then run:

```sh
python3 scripts/dev.py smoke --release --hardware --config config.local.toml --samples 3
```

Restore the service afterwards. Confirm the board and channel wiring before setting `hardware_confirmed = true`. A successful diagnostic proves valid pulse readings; it does not measure pump flow or prove moisture response. See [sensor configuration](sensors.md) and [hardware notes](hardware.md).

## Documentation checks

```sh
python3 -m venv .tools/docs
.tools/docs/bin/pip install -r requirements-docs.txt
.tools/docs/bin/zensical build --strict
.tools/docs/bin/python scripts/check_docs.py
.tools/docs/bin/zensical serve
```

Open the address printed by the preview server. Check navigation, search, code blocks and links. Zensical validates page links and anchors; the companion check verifies navigation coverage, README links and generated site links, including the project subpath. Documentation pull requests run these checks before deployment.

The documentation workflow builds pull requests and publishes successful `main` builds to [GitHub Pages](https://damacus.github.io/growhat-rs/). Documentation publication does not require a controller release tag. Keep every Markdown page in the navigation defined by `zensical.toml`.
