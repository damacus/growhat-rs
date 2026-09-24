# Measuring watering feedback

The Grow moisture probes report pulse frequency: lower Hz means wetter soil. Their readings are relative to the probe, soil and position, as described in Pimoroni's [Grow assembly and calibration guide](https://learn.pimoroni.com/article/assembling-grow). Do not infer a target percentage from the current uncalibrated frequencies.

Before enabling closed-loop watering, measure a dry reference and a wet-enough target for sensor 1 in its planted position. Keep the sensor away from the direct water stream. Record the actual mass delivered by each pump pulse and how long the sensor takes to respond. The observed pump flow near 50% was approximately 11.3 g/s in one trial; use the scale again because tubing prime and head height change delivery.

The read-only monitor can run on the Mac while the existing service stays on the Pi:

```sh
.tools/bin/python scripts/water_feedback.py --channel 1 --seconds 300 > .local/water-trial.csv
```

It prints timestamped raw Hz, status and pump acknowledgements. Once a *measured* wet-enough frequency is known, pass `--target-hz VALUE`. The monitor reports `target_reached` only after three healthy readings at or below that target; after a completed MQTT pump command it waits 60 seconds before evaluating the target. It never starts or stops a pump. A local CLI pulse does not emit an MQTT acknowledgement, so note its time separately if using the CLI.

For the first attended trial, collect a baseline, give a short pulse at a duty that reliably primes the tubing, wait for water to spread through the soil, then inspect the frequency trend and scale before repeating. Pimoroni recommends roughly 0.5-second pulses and 30–60 seconds between them in its [auto-watering guide](https://learn.pimoroni.com/article/auto-watering-with-grow). The measured response, rather than a low PWM percentage alone, should determine the pulse and wait pattern.

The later controller should pair pump 1 with sensor 1, stop on a measured target after repeated healthy samples, and stop with a fault on stale or invalid data, no response, excess delivered mass/time or restart. Automatic watering remains disabled until those limits and the sensor target are commissioned.
