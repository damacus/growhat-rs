#!/usr/bin/env python3
"""Send a temporary display message through the controller's configured MQTT broker."""

import argparse
import json
from pathlib import Path
import threading
import tomllib
import uuid

import paho.mqtt.client as mqtt


def message_fits(text):
    line, width = 1, 0
    for char in text:
        if char == "\n" or width == 24:
            line += 1
            width = 0
            if line > 5:
                return False
            if char == "\n":
                continue
        width += 1
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("text", help="Printable ASCII text, up to 120 bytes; newlines allowed")
    parser.add_argument("--config", type=Path, default=Path("config.local.toml"))
    parser.add_argument("--seconds", type=int, default=30, help="Display duration, 1–300 seconds")
    args = parser.parse_args()
    if (not args.text.strip() or len(args.text.encode()) > 120
            or any(c != "\n" and not " " <= c <= "~" for c in args.text)
            or not message_fits(args.text) or not 1 <= args.seconds <= 300):
        parser.error("Use printable ASCII fitting five 24-character lines and --seconds 1–300")
    settings = tomllib.loads(args.config.read_text())
    broker = settings["mqtt"]
    base = f"{broker.get('topic_prefix', 'growhat')}/{settings['device_id']}/display"
    client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id="growhat-display-" + uuid.uuid4().hex[:12])
    if "username" in broker:
        password = ""
        if "password_file" in broker:
            path = Path(broker["password_file"])
            if not path.is_absolute():
                path = args.config.parent / path
            password = path.read_text().rstrip("\r\n")
        client.username_pw_set(broker["username"], password)
    subscribed = threading.Event()
    acknowledged = threading.Event()
    responses = []

    def on_connect(client, _userdata, _flags, reason, _properties):
        if reason == 0:
            client.subscribe(base + "/ack", qos=1)

    def on_subscribe(_client, _userdata, _mid, reasons, _properties):
        if reasons and all(reason.value < 128 for reason in reasons):
            subscribed.set()

    def on_message(_client, _userdata, message):
        try:
            response = json.loads(message.payload)
            if not message.retain and response.get("text") == args.text:
                responses.append(response)
                acknowledged.set()
        except (ValueError, AttributeError):
            pass

    client.on_connect = on_connect
    client.on_subscribe = on_subscribe
    client.on_message = on_message
    client.connect_async(broker["host"], broker.get("port", 1883), 10)
    client.loop_start()
    try:
        if not subscribed.wait(10):
            raise RuntimeError("Could not subscribe to display acknowledgements")
        client.publish(base + "/set", json.dumps({"text": args.text, "ttl_seconds": args.seconds}),
                       qos=1, retain=False).wait_for_publish(5)
        if not acknowledged.wait(10):
            raise RuntimeError("No matching acknowledgement from controller")
        print(json.dumps(responses[-1]))
        if not responses[-1].get("accepted"):
            raise RuntimeError("Controller rejected the display message")
    finally:
        client.disconnect()
        client.loop_stop()


if __name__ == "__main__":
    main()
