#!/usr/bin/env python3
"""Send one bounded Grow HAT pump request and wait for its MQTT acknowledgement."""

import argparse
import json
from pathlib import Path
import threading
import tomllib

import paho.mqtt.client as mqtt


ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "config.local.toml")
    parser.add_argument("--token-file", type=Path, default=ROOT / ".local/pump-mqtt.token")
    parser.add_argument("--broker-host", help="Override the config's broker host for local DNS troubleshooting")
    parser.add_argument("--request-id", required=True, help="Reuse the same ID when retrying a request")
    parser.add_argument("--channel", type=int, required=True, choices=(1, 2, 3))
    parser.add_argument("--duty-percent", type=int, required=True, choices=range(1, 91), metavar="1..90")
    parser.add_argument("--duration-ms", type=int, required=True)
    args = parser.parse_args()
    if not 1 <= args.duration_ms <= 20_000:
        parser.error("--duration-ms must be between 1 and 20000")

    config = tomllib.loads(args.config.read_text())
    broker = config["mqtt"]
    host = args.broker_host or broker["host"]
    base = f"{broker.get('topic_prefix', 'growhat')}/{config['device_id']}"
    token = args.token_file.read_text().strip()
    command = {
        "request_id": args.request_id,
        "channel": args.channel,
        "duty_percent": args.duty_percent,
        "duration_ms": args.duration_ms,
        "token": token,
    }
    ready = threading.Event()
    replied = threading.Event()
    answers = []
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2)
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
            connection.subscribe(f"{base}/pump/ack")
            ready.set()

    def on_message(_connection, _userdata, message):
        answer = json.loads(message.payload)
        if answer.get("request_id") == args.request_id:
            answers.append(answer)
            replied.set()

    client.on_connect = on_connect
    client.on_message = on_message
    client.connect(host, broker.get("port", 1883), 5)
    client.loop_start()
    try:
        if not ready.wait(5):
            raise RuntimeError("MQTT connection or acknowledgement subscription timed out")
        client.publish(f"{base}/pump/set", json.dumps(command), qos=1, retain=False).wait_for_publish(5)
        if not replied.wait(args.duration_ms / 1000 + 10):
            raise RuntimeError("pump acknowledgement timed out; retry with the same request ID")
        answer = answers[0]
        print(json.dumps(answer, sort_keys=True))
        if answer["status"] != "completed":
            raise SystemExit(1)
    finally:
        client.loop_stop()
        client.disconnect()


if __name__ == "__main__":
    main()
