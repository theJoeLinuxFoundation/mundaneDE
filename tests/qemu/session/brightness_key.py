"""Send a laptop brightness key from a short-lived keyboard inside the test guest."""

import fcntl
import os
import struct
import subprocess
import sys
import time


# Linux uinput ABI; QEMU's QKeyCode enumeration has no brightness keys.
def main():
    brightness = {"up": 225, "down": 224}[sys.argv[1]]
    with os.fdopen(3, "wb", buffering=0) as keyboard:
        fcntl.ioctl(keyboard, 0x40045564, 1)  # UI_SET_EVBIT, EV_KEY
        for key in [*range(1, 59), brightness]:
            fcntl.ioctl(keyboard, 0x40045565, key)  # UI_SET_KEYBIT
        descriptor = struct.pack("80sHHHHI", b"mundane guest keyboard", 3, 1, 1, 1, 0)
        keyboard.write(descriptor + bytes(4 * 64 * 4))
        fcntl.ioctl(keyboard, 0x5501)  # UI_DEV_CREATE
        subprocess.run(["udevadm", "settle"], check=True)
        time.sleep(0.3)
        for pressed in (1, 0):
            os.write(
                keyboard.fileno(), struct.pack("llHHi", 0, 0, 1, brightness, pressed)
            )
            os.write(keyboard.fileno(), struct.pack("llHHi", 0, 0, 0, 0, 0))
            time.sleep(0.03)


if __name__ == "__main__":
    main()
