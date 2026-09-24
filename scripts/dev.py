#!/usr/bin/env python3
"""Cached cross-builds and isolated, bounded Pi diagnostics (Python 3.11+)."""

import argparse
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / ".tools"
TARGET = "arm-unknown-linux-gnueabihf"
RUST = "1.98.1"
ZIG = "0.13.0"
ZIGBUILD = "0.20.1"
REMOTE = ".local/growhat-dev"


def run(args, **kwargs):
    print("+ " + shlex.join(map(str, args)), file=sys.stderr)
    return subprocess.run(list(map(str, args)), check=True, cwd=ROOT, **kwargs)


def build_env():
    env = os.environ.copy()
    env.update({
        "RUSTUP_TOOLCHAIN": RUST,
        "CARGO_ZIGBUILD_PYTHON_PATH": str(TOOLS / "bin/python"),
        "CARGO_ZIGBUILD_CACHE_DIR": str(TOOLS / "cache/cargo-zigbuild"),
        "ZIG_GLOBAL_CACHE_DIR": str(TOOLS / "cache/zig"),
        "ZIG_LOCAL_CACHE_DIR": str(TOOLS / "cache/zig-local"),
    })
    return env


def setup():
    if not (TOOLS / "bin/python").exists():
        run(["uv", "venv", TOOLS, "--python", "3.12"])
    run(["uv", "pip", "install", "--python", TOOLS / "bin/python",
         f"ziglang=={ZIG}", f"cargo-zigbuild=={ZIGBUILD}", "paho-mqtt==2.1.0"])
    run(["rustup", "toolchain", "install", RUST, "--profile", "minimal",
         "--component", "rustfmt,clippy"])
    run(["rustup", "target", "add", TARGET, "--toolchain", RUST])


def build(release):
    compiler = TOOLS / "bin/cargo-zigbuild"
    if not compiler.exists():
        raise ValueError("Build tools missing. Run: python3 scripts/dev.py setup")
    run([compiler, "zigbuild", "--locked", "--target", TARGET + ".2.28"]
        + (["--release"] if release else ["--profile", "pi-dev"]), env=build_env())
    return ROOT / "target" / TARGET / ("release" if release else "pi-dev") / "growhat"


def ssh_options(control_path, identity=None):
    options = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=8",
            "-o", "StrictHostKeyChecking=yes", "-o", "ServerAliveInterval=5",
            "-o", "ServerAliveCountMax=2", "-o", f"ControlPath={control_path}"]
    if identity is not None:
        options += ["-i", str(identity), "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none"]
    return options


@contextmanager
def ssh_connection(host, identity=None):
    # OpenSSH's UNIX socket path limit is short, so keep this directory under /tmp.
    with tempfile.TemporaryDirectory(prefix="growhat-ssh-", dir="/tmp") as directory:
        control_path = Path(directory) / "socket"
        try:
            run(["ssh", *ssh_options(control_path, identity), "-o", "ControlMaster=yes",
                 "-Nf", host], timeout=20)
            yield control_path
        finally:
            # A failed transfer or diagnostic must not leave a master behind.
            try:
                subprocess.run(["ssh", "-o", "BatchMode=yes", "-o",
                                f"ControlPath={control_path}", "-O", "exit", host],
                               check=False, capture_output=True, timeout=10)
            except subprocess.TimeoutExpired:
                pass


def ssh(host, command, control_path, identity=None, **kwargs):
    return run(["ssh", *ssh_options(control_path, identity), "-o", "ControlMaster=auto",
                host, command], **kwargs)


def copy_changed(host, source, name, control_path, identity=None):
    dest = f"{REMOTE}/{name}"
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    result = ssh(host, f"if test -f {dest}; then sha256sum {dest}; fi", control_path,
                 identity, capture_output=True, text=True, timeout=20)
    if result.stdout.split()[:1] == [digest]:
        print(f"Unchanged: {name}", file=sys.stderr)
        return
    staged = dest + ".new"
    run(["scp", "-q", *ssh_options(control_path, identity), "-o", "ControlMaster=auto",
         source, f"{host}:{staged}"], timeout=90)
    verify = f"test \"$(sha256sum {staged} | cut -d ' ' -f 1)\" = {digest}"
    if name == "growhat":
        promote = (f"chmod 755 {staged} && {staged} --version && "
                   f"if test -f {dest}; then cp -p {dest} {dest}.previous; fi && "
                   f"mv {staged} {dest}")
    else:
        promote = f"chmod 600 {staged} && mv {staged} {dest}"
    ssh(host, f"{verify} && {promote}", control_path, identity, timeout=20)


def smoke(args):
    if not re.fullmatch(r"[a-zA-Z0-9_][a-zA-Z0-9_.-]*@[a-zA-Z0-9][a-zA-Z0-9.-]*", args.host):
        raise ValueError("Use an SSH destination such as pi@192.168.1.216")
    config = args.config.resolve()
    identity = args.identity
    local_key = ROOT / ".local/ssh/growhat_pi_ed25519"
    if identity is None and args.host == "pi@192.168.1.216" and local_key.is_file():
        identity = local_key
    if identity is not None:
        identity = identity.resolve(strict=True)
    settings = tomllib.loads(config.read_text())
    if settings.get("backend", "simulated") == "hardware" and not args.hardware:
        raise ValueError("Hardware diagnostics require --hardware and hardware_confirmed=true in the config")
    if not 1 <= args.samples <= 100:
        raise ValueError("--samples must be between 1 and 100")
    interval = settings.get("sample_interval_ms", 5000)
    window = settings.get("measurement_window_ms", 1000)
    # Bound both remote execution and the SSH client. The application validates
    # configuration before touching GPIO. A bad config cannot remove the timeout.
    seconds = min(3600, max(10, int(args.samples * (interval + window) / 1000) + 10))
    start = time.monotonic()
    binary = build(args.release)
    built = time.monotonic()
    with ssh_connection(args.host, identity) as control_path:
        ssh(args.host, f"umask 077; mkdir -p {REMOTE}", control_path, identity, timeout=20)
        copy_changed(args.host, binary, "growhat", control_path, identity)
        copy_changed(args.host, config, "diagnostic.toml", control_path, identity)
        transferred = time.monotonic()
        ssh(args.host, f"timeout --signal=TERM --kill-after=3 {seconds}s "
            f"{REMOTE}/growhat --config {REMOTE}/diagnostic.toml diagnose --samples {args.samples}",
            control_path, identity, timeout=seconds + 20)
    finished = time.monotonic()
    print(f"Build {built-start:.2f}s; transfer {transferred-built:.2f}s; "
          f"diagnostic {finished-transferred:.2f}s; total {finished-start:.2f}s", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("setup", help="Install pinned local build tools and the Rust target")
    build_parser = sub.add_parser("build", help="Cross-compile for the Pi Zero")
    build_parser.add_argument("--release", action="store_true")
    smoke_parser = sub.add_parser("smoke", help="Build, copy changed files and run bounded diagnostics")
    smoke_parser.add_argument("--host", default="pi@192.168.1.216")
    smoke_parser.add_argument("--identity", type=Path, help="Use a local SSH key without an agent")
    smoke_parser.add_argument("--config", type=Path, default=ROOT / "examples/simulated.toml")
    smoke_parser.add_argument("--samples", type=int, default=3)
    smoke_parser.add_argument("--release", action="store_true")
    smoke_parser.add_argument("--hardware", action="store_true", help="Allow an explicitly confirmed hardware configuration")
    args = parser.parse_args()
    if args.command == "setup":
        setup()
    elif args.command == "build":
        print(build(args.release))
    else:
        smoke(args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"Error: {error}", file=sys.stderr)
        sys.exit(1)
