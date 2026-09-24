#!/usr/bin/env python3
"""Observe Grow moisture response to manual pump pulses without actuating pumps."""

import argparse
import csv
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import queue
import sys
import threading
import time
import tomllib
import uuid

import paho.mqtt.client as mqtt


ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "config.local.toml")
    parser.add_argument("--channel", type=int, default=1, choices=(1, 2, 3))
    parser.add_argument("--seconds", type=int, default=180)
    parser.add_argument("--target-hz", type=float,
                        help="Measured frequency at which the soil is wet enough; lower is wetter")
    parser.add_argument("--stable-samples", type=int, default=3)
    parser.add_argument("--settle-seconds", type=int, default=60)
    args = parser.parse_args()
    if not 5 <= args.seconds <= 3600:
        parser.error("--seconds must be between 5 and 3600")
    if args.target_hz is not None and (not math.isfinite(args.target_hz) or args.target_hz <= 0):
        parser.error("--target-hz must be a positive finite number")
    if not 1 <= args.stable_samples <= 100 or not 0 <= args.settle_seconds <= 600:
        parser.error("stable samples must be 1..100 and settle seconds 0..600")

    config = tomllib.loads(args.config.read_text())
    broker = config["mqtt"]
    base = f"{broker.get('topic_prefix', 'growhat')}/{config['device_id']}"
    state_topic = f"{base}/{args.channel}/state"
    ack_topic = f"{base}/pump/ack"
    events = queue.Queue()
    connected = threading.Event()
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2,
                         client_id="growhat-water-feedback-" + uuid.uuid4().hex[:12])
    if broker.get("username"):
        password = ""
        if broker.get("password_file"):
            path = Path(broker["password_file"])
            if not path.is_absolute():
                path = args.config.parent / path
            password = path.read_text().rstrip("\r\n")
        client.username_pw_set(broker["username"], password)

    def on_connect(connection, _userdata, _flags, reason, _properties):
        if reason == 0:
            connection.subscribe([(state_topic, 1), (ack_topic, 1)])
            connected.set()

    def on_message(_connection, _userdata, message):
        if not message.retain:
            events.put((time.monotonic(), message.topic, message.payload))

    client.on_connect = on_connect
    client.on_message = on_message
    client.connect_async(broker["host"], broker.get("port", 1883), 10)
    client.loop_start()
    writer = csv.writer(sys.stdout)
    writer.writerow(["time_utc", "elapsed_s", "event", "status", "raw_hz", "age_ms",
                     "request_id", "decision"])
    sys.stdout.flush()
    started = time.monotonic()
    last_pulse = None
    consecutive = 0
    try:
        if not connected.wait(10):
            raise RuntimeError("Could not connect to MQTT broker")
        while (remaining := args.seconds - (time.monotonic() - started)) > 0:
            try:
                received_at, topic, payload = events.get(timeout=min(1, remaining))
            except queue.Empty:
                continue
            try:
                data = json.loads(payload)
            except (ValueError, UnicodeDecodeError):
                continue
            event = "sensor" if topic == state_topic else "pump_ack"
            status = data.get("status", "")
            raw_hz = data.get("raw_hz") if event == "sensor" else None
            age_ms = data.get("age_ms") if event == "sensor" else None
            request_id = data.get("request_id", "") if event == "pump_ack" else ""
            decision = ""
            if event == "pump_ack" and status == "completed":
                last_pulse = received_at
                consecutive = 0
            elif event == "sensor" and args.target_hz is not None:
                healthy = (status in ("valid", "uncalibrated")
                           and isinstance(raw_hz, (int, float))
                           and math.isfinite(raw_hz)
                           and isinstance(age_ms, (int, float))
                           and age_ms < config.get("stale_after_ms", 15000))
                if not healthy:
                    consecutive = 0
                    decision = "invalid"
                elif last_pulse is not None and received_at - last_pulse < args.settle_seconds:
                    consecutive = 0
                    decision = "settling"
                elif raw_hz <= args.target_hz:
                    consecutive += 1
                    decision = ("target_reached" if consecutive >= args.stable_samples
                                else "checking_target")
                else:
                    consecutive = 0
                    decision = "drier_than_target"
            writer.writerow([datetime.now(timezone.utc).isoformat(timespec="seconds"),
                             round(received_at - started, 1), event, status, raw_hz, age_ms,
                             request_id, decision])
            sys.stdout.flush()
    finally:
        client.disconnect()
        client.loop_stop()


if __name__ == "__main__":
    main()
