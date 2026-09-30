# Display and buttons

Set top-level `panel_enabled = true` in a confirmed hardware configuration. This uses the Grow HAT Mini's existing `/dev/spidev0.0`, DC GPIO9 and backlight GPIO12, plus pull-up button inputs A=5, B=6, X=16 and Y=24. The service needs the `spi` and `gpio` groups and access to both device nodes. The packaged service includes these permissions. Stop other programs using the display or buttons before enabling it.

## Local readings

The normal LCD header shows the Pi's IPv4 address and a rolling sample counter, for example `IP 192.168.1.216 #0042`. The address is selected from the default network route without DNS or an MQTT connection and is checked every five seconds. It shows `IP unavailable` when no address can be selected. Temporary display messages hide the header until they expire.

The normal screen shows moisture readings, button press counts and a four-digit sample counter beside the IP address. The counter advances every sensor cycle even when rounded readings stay the same, making a stalled screen visible. Button states are debounced for 30ms; counts and the sample counter reset when the process restarts. Held-state and press-count entities use the original Python button entity identities, so they return under the same Home Assistant device. No watering action is bound to a button.

## Remote button controls

Home Assistant also discovers `Press A/B/X/Y on HAT` button controls on the same device. Pressing one sends a non-retained `PRESS` command to `<topic_prefix>/<device_id>/buttons/<letter>/press/set`; the controller increments that button's press count, shows `REMOTE <letter>` briefly and publishes an acknowledgement under `/press/ack`. These virtual presses do not energise a pump or claim that a physical button is held. Limit publish access to these topics if other broker clients should not be able to trigger them.

## Temporary display messages

Send a temporary test message from the development host:

```sh
.tools/bin/python scripts/display.py "RUST DISPLAY TEST" --seconds 30 --config config.local.toml
```

The controller accepts non-retained JSON on `<topic_prefix>/<device_id>/display/set`:

```json
{"text":"RUST DISPLAY TEST","ttl_seconds":30}
```

Messages accept printable ASCII and newlines, fitting five lines of 24 characters (up to 120 bytes), and last 1–300 seconds (default 30). Letters render in uppercase. The screen returns to moisture readings afterwards. `<topic_prefix>/<device_id>/display/ack` reports whether the message was queued; it is not visual proof of the LCD. The device logs successful frame writes. Messages are rejected if the panel is disabled, the input is invalid or its queue is full. Use the broker's normal access controls; anyone allowed to publish to this command topic can change its temporary screen message.
