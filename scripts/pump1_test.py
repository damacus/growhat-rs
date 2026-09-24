"""One-shot, attended Grow HAT Mini pump 1 test. Never used by the service."""

import signal
import time

import RPi.GPIO as GPIO


PIN = 17  # Pimoroni Grow HAT Mini pump 1
FREQUENCY_HZ = 10_000  # Pimoroni reference driver
PULSES = ((10, 1.0), (50, 1.0))


def interrupted(_signum, _frame):
    raise InterruptedError("pump test interrupted")


def main():
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)

    GPIO.setmode(GPIO.BCM)
    GPIO.setup(PIN, GPIO.OUT, initial=GPIO.LOW)
    pwm = None
    try:
        pwm = GPIO.PWM(PIN, FREQUENCY_HZ)
        pwm.start(0)
        for index, (duty, seconds) in enumerate(PULSES):
            pwm.ChangeDutyCycle(duty)
            print(f"Pump 1: {duty}% for {seconds:.1f}s", flush=True)
            time.sleep(seconds)
            pwm.ChangeDutyCycle(0)
            if index + 1 < len(PULSES):
                time.sleep(0.5)
    finally:
        if pwm is not None:
            pwm.ChangeDutyCycle(0)
            pwm.stop()
        GPIO.output(PIN, GPIO.LOW)
        GPIO.setup(PIN, GPIO.IN)
        GPIO.cleanup(PIN)
        print("Pump 1: off", flush=True)


if __name__ == "__main__":
    main()
