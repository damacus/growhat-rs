# Development

Prerequisites: Rust/rustup, Python 3.11+, [uv](https://docs.astral.sh/uv/), SSH and SCP. Setup pins Rust 1.98.1, Zig 0.13.0 and cargo-zigbuild 0.20.1; Python tools and compiler caches stay in ignored `.tools/`.

```sh
python3 scripts/dev.py setup
python3 scripts/dev.py smoke
```

The smoke command cross-compiles `arm-unknown-linux-gnueabihf` for glibc 2.28, copies changed files to `pi@192.168.1.216:~/.local/growhat-dev/`, then runs three simulated samples under a remote timeout. It prints build, transfer and diagnostic timings. It uses the existing SSH host trust and key; connect interactively and verify the host key first if it is unknown.

## SSH and staged binaries

For a dedicated local development key, use `--identity PATH`. The default Pi automatically uses ignored `.local/ssh/growhat_pi_ed25519` when present, with the SSH agent disabled. Creating a local key does not authorise it remotely: install its public key on the Pi through an existing authenticated connection first. Temporary key expiry and revocation details are kept alongside the local key.

No Rust compiler is installed on the Pi. The command uses cached, incremental `pi-dev` builds without debug information to reduce transfer size. Native development builds retain normal debugging information. One SSH connection is reused for the invocation and closed afterwards. The command checks file hashes, stages the executable before replacing it, and saves `growhat.previous`. It does not install or restart a system service. Only one smoke invocation should target this directory at a time.

## Hardware diagnostics

```sh
# Optimised binary for device CPU/memory/timing checks:
python3 scripts/dev.py build --release
# A confirmed, local hardware config (stop any service owning its GPIO first):
python3 scripts/dev.py smoke --release --hardware --config config.local.toml --samples 3
```

Restore any stopped hardware service after the diagnostic. The smoke command does not manage service state.

The confirmed target is a Pi Zero W Rev 1.1 (ARMv6), 32-bit Raspbian 13 with glibc 2.41. ARMv7 and AArch64 binaries are incompatible with this device. See [hardware notes](hardware.md) and [validation status](validation.md).

The controller is tested and continuously exercised on the local Pi Zero W. See the dated [validation records](validation.md) for previous deployment checks and the [Python baseline](python-rust-baseline.md) for measurements and migration history.

## Native simulation

Use the simulated configuration for fast checks on your development host:

```sh
cargo run --locked -- --config examples/simulated.toml check-config
cargo run --locked -- --config examples/simulated.toml diagnose --samples 3
```

Diagnostics emit one JSON object per sensor per sample to stdout; logs go to stderr. `diagnose` never connects to MQTT. Exit codes are 0 for success, 1 for runtime or reading failures, and 2 for invalid arguments or configuration. `check-config` validates without opening GPIO or contacting a broker.

See [testing](testing.md) for the full software and Pi checks.
