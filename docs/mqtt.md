# MQTT and Home Assistant

The hardware example already contains an `[mqtt]` table. Edit that table for your broker rather than adding a second one:

```toml
[mqtt]
host = "mqtt.example.lan"
port = 1883
topic_prefix = "growhat"
discovery_prefix = "homeassistant"
username = "growhat"
password_file = "mqtt.credentials"
```

Put only the password in the credential file and restrict its permissions to the service user. Relative credential paths resolve beside the TOML file. Do not pass passwords as command arguments or commit them. Anonymous brokers can omit both credential fields.

The `run` command is a foreground commissioning check. Stop an existing controller before running another hardware instance, then restore the service afterwards. See [service installation](installation.md) for normal operation.

```sh
sudo -u growhat /opt/growhat/growhat --config /etc/growhat/config.toml check-config
sudo -u growhat /opt/growhat/growhat --config /etc/growhat/config.toml run
```

This version uses MQTT 3.1.1 over TCP on a trusted network; TLS is not implemented. Use a broker that accepts this protocol. Set a unique, stable `device_id` for each controller. A simulated controller must use a different ID from real hardware.

## Discovery and entity identity

Home Assistant discovery creates a visible frequency sensor and diagnostic status entity for every configured channel, plus moisture percentage when calibration is present. Frequency is shown to two decimal places while the MQTT payload keeps the full measured value. The previous diagnostic `raw` discovery identity is removed on startup; its old recorder history stays under the old entity ID. Removing calibration clears its retained moisture discovery entry. Other removed channels/devices require removing their retained discovery topics manually; changing IDs intentionally creates new entities.

During the September 2026 migration, the old Python discovery entries `Moisture 1/2/3 frequency` and `Illuminance` were removed from the broker after their retained configuration was backed up. This was a migration step, not automatic startup behaviour. Restoring the Python publisher can restore those entries. Use the Rust frequency entities for current readings. A channel's frequency entity is unavailable while its diagnostic status is `no_signal`, `invalid` or `stale`; the status JSON includes the sensor error.

## Availability and reconnects

Discovery is retained. State is not retained, and numeric entities require both device availability and a healthy reading. `expire_after` prevents a stalled sampler appearing current. The status entity exposes `no_signal`, `invalid` or `stale` and JSON attributes include sample age and error. The app republishes discovery after broker reconnect and the default Home Assistant birth message (`homeassistant/status`, or the configured discovery prefix plus `/status`). The Last Will reports offline after connection loss; SIGINT/SIGTERM also publish offline.

An outage retains only the latest sensor snapshot in memory. Reconnection discards the failed MQTT client's pending requests and reevaluates snapshot age before publishing.
