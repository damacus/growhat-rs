# Hardware assumptions and sensor-only access

## Target inspected and commissioned

- `pi@192.168.1.216` (formerly `.213`): Raspberry Pi Zero W Rev 1.1, BCM2835, ARMv6, 32-bit Raspbian 13, kernel 6.18.50+rpt-rpi-v6 at handover, glibc 2.41.
- Device nodes observed: `/dev/gpiochip0`, `/dev/gpiochip4`, `/dev/i2c-1`, `/dev/i2c-2`, `/dev/spidev0.0`.
- No HAT identity was returned from `/proc/device-tree/hat`, so the exact PCB revision remains unknown. The existing working Python driver uses the manufacturer's Grow mapping below. A 23 September Rust diagnostic read all three connected channels: channel 1 around 10.49Hz and channels 2 and 3 around 6.33Hz.
- `/dev/gpiochip0` is `pinctrl-bcm2835`; `/dev/gpiochip4` resolves to the same chip. The Rust diagnostic successfully requested all three channels as inputs.

## Reference wiring

Pimoroni's [Grow product page](https://shop.pimoroni.com/products/grow) identifies the sensors as pulse-frequency output, approximately 2–30Hz. The official [moisture driver](https://github.com/pimoroni/grow-python/blob/main/grow/moisture.py) maps channels 1, 2 and 3 to BCM GPIO 23, 8 and 25. Its [Grow HAT Mini schematic](https://cdn.shopify.com/s/files/1/0174/1800/files/grow-mini_1.pdf?v=1600422759) is linked from the manufacturer page; the implementer also checked a [schematic mirror](https://www.electrokit.com/upload/product/41018/41018071/grow-mini_1.pdf).

| Channel | BCM GPIO | Physical header pin |
|---|---:|---:|
| 1 | 23 | 16 |
| 2 | 8 | 24 |
| 3 | 25 | 22 |

The Rust driver requests only these configured lines as rising-edge inputs through GPIO character-device ABI v2. GPIO chip offsets must match BCM numbering. Confirm the chip label and line names with `gpioinfo` before setting `hardware_confirmed = true`. No I2C scanning, pump lines, display, buzzer or output requests are needed.

GPIO8 can conflict with SPI CE0. Pimoroni's [reference installation notes](https://github.com/pimoroni/grow-python#or-install-from-pypi-and-configure-manually) describe `dtoverlay=spi0-cs,cs0_pin=14` for the Grow display wiring. Inspect the current boot configuration and line owner if GPIO8 is busy; do not automatically change overlays or release another driver's lines.

## Commissioning still needed

1. Deliberately change each probe's condition and confirm its raw frequency responds; steady readings prove pulse input, not moisture response.
2. Measure calibration endpoints for each actual probe and plant setup, if percentages are desired.
3. Check the probe or cable originally plugged into channel 2. After swapping channels 2 and 3, channel 2 produced pulses and channel 3 had none in three consecutive windows. The fault followed that probe or cable. Reseat or replace it, then repeat the diagnostic.

The existing boot overlay is `dtoverlay=spi0-1cs,cs0_pin=7,no_miso`; it was left unchanged. See the [baseline and handover record](python-rust-baseline.md). Stop the running sensor service before requesting its GPIO lines from a separate diagnostic.

Pump 1 gave two attended, one-second test pulses through the Grow HAT output. The Rust service has no pump-control path. Pump model/electrical margin and delivered flow still need verification before watering is enabled.

## Optional display and buttons

The display follow-up ports the archived, previously working Python settings: ST7735 on `/dev/spidev0.0`, SPI mode 0 at 4MHz, DC=BCM9, backlight=BCM12, 160x80 image rotated 270 degrees, panel offsets 26/1. Buttons A/B/X/Y are active-low inputs on BCM5/6/16/24 with pull-ups and 30ms debounce. These lines are requested only when `panel_enabled = true` and the hardware backend is confirmed. The existing `cs0_pin=7,no_miso` overlay leaves BCM8 available for a future channel 2 probe and BCM9 for display DC; do not replace that overlay blindly.

This support is deployed on the Pi. The Rust service logs a successful SPI frame write for `RUST DISPLAY TEST`, and MQTT returns the four released button snapshots. Physical LCD appearance and a real button press still need a person beside the Pi to confirm.
