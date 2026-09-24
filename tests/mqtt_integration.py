#!/usr/bin/env python3
"""Real Mosquitto checks; run with .tools/bin/python tests/mqtt_integration.py."""

import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading
import time
import uuid

import paho.mqtt.client as mqtt

ROOT = Path(__file__).resolve().parents[1]
IMAGE = "eclipse-mosquitto@sha256:38c0da4f2ef84284d47b3b3eeea1cb3bdeabe81ee10caf0cd5c5ff61ee3ea408"


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True, stderr=subprocess.PIPE, timeout=45).strip()


def delayed_connack_proxy(broker_port):
    """Forward one connection, delaying the broker's first response past sample intervals."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    proxy_port = listener.getsockname()[1]
    done = threading.Event()

    def relay(source, destination, delay=0):
        try:
            if delay:
                time.sleep(delay)
            while not done.is_set():
                data = source.recv(65536)
                if not data:
                    break
                destination.sendall(data)
        except OSError:
            pass
        finally:
            done.set()
            source.close()
            destination.close()

    def accept():
        client, _ = listener.accept()
        listener.close()
        broker = socket.create_connection(("127.0.0.1", broker_port))
        threading.Thread(target=relay, args=(client, broker), daemon=True).start()
        threading.Thread(target=relay, args=(broker, client, 1.5), daemon=True).start()

    threading.Thread(target=accept, daemon=True).start()
    return proxy_port, listener


def main():
    subprocess.run(["cargo", "build", "--locked"], cwd=ROOT, check=True)
    (ROOT / ".local").mkdir(exist_ok=True)
    name = "growhat-test-" + uuid.uuid4().hex[:10]
    app = None
    observer = None
    with tempfile.TemporaryDirectory(prefix="mqtt-", dir=ROOT / ".local") as temp:
        temp = Path(temp)
        broker_config = temp / "mosquitto.conf"
        broker_config.write_text("listener 1883\nallow_anonymous true\npersistence false\n")
        broker_config.chmod(0o644)
        os.chmod(temp, 0o755)
        try:
            # Docker can allocate a different ephemeral host port on restart.
            # Reserve a candidate and ask Docker to keep that explicit mapping.
            with socket.socket() as candidate:
                candidate.bind(("127.0.0.1", 0))
                port = candidate.getsockname()[1]
            docker("run", "-d", "--rm", "--name", name, "-p", f"127.0.0.1:{port}:1883",
                   "--mount", f"type=bind,source={broker_config},target=/mosquitto/config/mosquitto.conf,readonly", IMAGE)
            messages = queue.Queue()
            connected = threading.Event()
            observer = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2, client_id=name)

            def on_connect(client, _userdata, _flags, reason, _properties):
                assert reason == 0, reason
                client.subscribe("#")
                connected.set()

            observer.on_connect = on_connect
            observer.on_disconnect = lambda *_: connected.clear()
            observer.on_message = lambda _c, _u, m: messages.put((m.topic, m.payload, m.retain))
            observer.connect_async("127.0.0.1", port, 5)
            observer.loop_start()
            assert connected.wait(10), "observer could not connect to broker"

            def wait(topic, predicate=lambda _: True, seconds=15):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    try:
                        message = messages.get(timeout=max(0.01, deadline-time.monotonic()))
                    except queue.Empty:
                        break
                    if message[0] == topic and predicate(message[1]):
                        return message
                log = (temp / "app.log").read_text() if (temp / "app.log").exists() else ""
                raise AssertionError(f"Timed out waiting for {topic}\n{log}")

            def clear():
                while True:
                    try:
                        messages.get_nowait()
                    except queue.Empty:
                        return

            config = temp / "config.toml"
            config.write_text(f'''device_id = "integration_test"
name = "Integration test"
backend = "simulated"
sample_interval_ms = 200
measurement_window_ms = 100
stale_after_ms = 1000
[[sensors]]
channel = 1
name = "Soil"
simulated_hz = 20.0
[sensors.calibration]
dry_hz = 30.0
wet_hz = 10.0
[mqtt]
host = "127.0.0.1"
port = {port}
''')
            with (temp / "app.log").open("w") as log:
                def start_app():
                    return subprocess.Popen([ROOT / "target/debug/growhat", "--config", config, "run"],
                                            stdout=subprocess.DEVNULL, stderr=log)

                app = start_app()
                discovery_topic = "homeassistant/sensor/integration_test_1/moisture/config"
                discovery = json.loads(wait(discovery_topic)[1])
                assert discovery["unique_id"] == "integration_test_1_moisture"
                assert discovery["expire_after"] == 1
                assert discovery["availability_mode"] == "all"
                assert len(discovery["availability"]) == 2
                state_topic = "growhat/integration_test/1/state"
                state_message = wait(state_topic, lambda b: json.loads(b)["status"] == "valid")
                state = json.loads(state_message[1])
                assert state["raw_hz"] == 20 and state["moisture_percent"] == 50
                assert not state_message[2], "telemetry must not be retained"
                print("PASS discovery, calibrated telemetry, availability and nonretained state")

                clear()
                observer.publish("growhat/integration_test/display/set",
                                 json.dumps({"text": "TEST PANEL", "ttl_seconds": 30}), qos=1).wait_for_publish(5)
                ack = json.loads(wait("growhat/integration_test/display/ack")[1])
                assert ack["accepted"] is False, ack
                assert ack["text"] == "TEST PANEL", ack
                assert app.poll() is None
                print("PASS disabled panel rejects display message without disrupting readings")

                clear()
                pump_topic = "growhat/integration_test/pump/set"
                pump_ack = "growhat/integration_test/pump/ack"
                command = {"request_id": "disabled-test", "channel": 1,
                           "duty_percent": 10, "duration_ms": 1000, "token": "wrong"}
                observer.publish(pump_topic, json.dumps(command), qos=1).wait_for_publish(5)
                assert json.loads(wait(pump_ack)[1])["status"] == "rejected"
                clear()
                observer.publish(pump_topic, json.dumps(command), qos=1, retain=True).wait_for_publish(5)
                assert json.loads(wait(pump_ack)[1])["status"] == "rejected"
                observer.publish(pump_topic, b"", qos=1, retain=True).wait_for_publish(5)
                print("PASS disabled and retained pump commands are rejected")

                clear()
                observer.publish("homeassistant/status", "online").wait_for_publish(5)
                assert json.loads(wait(discovery_topic)[1])["unique_id"] == discovery["unique_id"]
                print("PASS Home Assistant birth republishing with stable identity")

                clear()
                docker("restart", "-t", "1", name)
                assert app.poll() is None, "application exited during broker restart"
                # Persistence is disabled: discovery must be regenerated by the app.
                assert json.loads(wait(discovery_topic, seconds=25)[1])["unique_id"] == discovery["unique_id"]
                wait(state_topic, lambda b: json.loads(b)["status"] == "valid")
                print("PASS broker restart recovery")

                clear()
                app.terminate()
                assert app.wait(timeout=5) == 0
                wait("growhat/integration_test/availability", lambda b: b == b"offline")
                print("PASS graceful shutdown availability")

                clear()
                app = start_app()
                wait("growhat/integration_test/availability", lambda b: b == b"online")
                clear()
                app.kill()
                app.wait(timeout=5)
                wait("growhat/integration_test/availability", lambda b: b == b"offline")
                print("PASS process death Last Will")

                # Removing calibration must remove retained percentage discovery.
                config.write_text(config.read_text().replace("[sensors.calibration]\ndry_hz = 30.0\nwet_hz = 10.0\n", ""))
                clear()
                app = start_app()
                wait(discovery_topic, lambda b: b == b"")
                uncalibrated = json.loads(wait(state_topic, lambda b: json.loads(b)["status"] == "uncalibrated")[1])
                assert uncalibrated["raw_hz"] == 20 and uncalibrated["moisture_percent"] is None
                print("PASS missing calibration preserves raw readings and removes percentage entity")

                app.terminate()
                assert app.wait(timeout=5) == 0
                clear()
                proxy_port, proxy_listener = delayed_connack_proxy(port)
                try:
                    config.write_text(config.read_text().replace(f"port = {port}", f"port = {proxy_port}"))
                    app = start_app()
                    wait(state_topic, lambda b: json.loads(b)["status"] == "uncalibrated", seconds=8)
                    assert app.poll() is None
                    print("PASS delayed CONNACK despite 200ms sampling")
                finally:
                    proxy_listener.close()
        finally:
            if app is not None and app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait(timeout=5)
            if observer is not None:
                observer.disconnect()
                observer.loop_stop()
            try:
                docker("rm", "-f", name)
            except subprocess.CalledProcessError:
                pass


if __name__ == "__main__":
    main()
