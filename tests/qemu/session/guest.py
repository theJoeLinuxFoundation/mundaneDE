"""Drive the serial shell and QMP of a private session-test VM."""

import argparse
import json
from pathlib import Path
import re
import shlex
import socket
import uuid


class Guest:
    def __init__(self, run):
        self.run = Path(run).resolve(strict=True)

    def connect(self, name, timeout):
        sock = socket.socket(socket.AF_UNIX)
        sock.settimeout(timeout)
        sock.connect(str(self.run / name))
        return sock

    def qmp(self, command, **arguments):
        with self.connect("qmp", 10) as sock, sock.makefile("rwb", buffering=0) as wire:
            assert "QMP" in json.loads(wire.readline())
            for msg in (
                {"execute": "qmp_capabilities"},
                {"execute": command, "arguments": arguments},
            ):
                wire.write(json.dumps(msg).encode() + b"\n")
                while True:
                    reply = json.loads(wire.readline())
                    if "error" in reply:
                        raise RuntimeError(reply["error"])
                    if "return" in reply:
                        break
            return reply["return"]

    def sync_console(self):
        # The emulated UART can lose bytes just as the guest resumes. Wait
        # for the known shell prompt before sending a quoted command.
        for _ in range(12):
            try:
                with self.connect("serial", 1) as sock:
                    sock.sendall(b"\x03\n")
                    received = bytearray()
                    while b"joe# " not in received:
                        data = sock.recv(1024)
                        if not data:
                            break
                        received.extend(data)
                    if b"joe# " in received:
                        return
            except TimeoutError:
                continue
        raise TimeoutError("test-console prompt after wake")

    def shell(self, command, timeout=60, check=True):
        marker = uuid.uuid4().hex
        # Separate the marker from its prefix in the input so terminal echo
        # cannot satisfy the result check. Do not evaluate guest output.
        line = f"printf '\\nSTART%s\\n' {marker}; sh -c {shlex.quote(command)}; printf '\\nEND%s %d\\n' {marker} $?\n"
        with self.connect("serial", timeout) as sock:
            sock.sendall(line.encode())
            data = bytearray()
            end = re.compile(rb"\nEND" + marker.encode() + rb" (\d+)\r?\n")
            while not (match := end.search(data)):
                chunk = sock.recv(65536)
                if not chunk:
                    raise EOFError("guest serial console")
                data.extend(chunk)
        begin = data.index(b"\nSTART" + marker.encode())
        output = (
            data[data.index(b"\n", begin + 1) + 1 : match.start()]
            .decode(errors="replace")
            .replace("\r", "")
            .strip()
        )
        if check and int(match[1]):
            raise RuntimeError(f"guest command failed ({int(match[1])}): {command}\n{output}")
        return output

    def screen(self, filename):
        path = self.run / (filename + ".ppm")
        self.qmp("screendump", filename=str(path))
        return path.read_bytes()

    def move(self, x, y):
        # Coordinates are fractions of the single QEMU scanout.
        self.qmp(
            "input-send-event",
            events=[
                dict(type="abs", data=dict(axis=axis, value=round(value * 32767)))
                for axis, value in (("x", x), ("y", y))
            ],
        )

    def click(self, x, y):
        self.move(x, y)
        for down in (True, False):
            self.qmp(
                "input-send-event", events=[dict(type="btn", data=dict(button="left", down=down))]
            )

    def key(self, *keys):
        return self.qmp("send-key", keys=[dict(type="qcode", data=k) for k in keys])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path)
    parser.add_argument("operation", choices=["shell", "qmp", "key"])
    parser.add_argument("args", nargs="+")
    args = parser.parse_args()
    guest = Guest(args.run)
    if args.operation == "shell":
        print(guest.shell(" ".join(args.args)))
    elif args.operation == "key":
        print(guest.key(*args.args))
    else:
        print(
            json.dumps(
                guest.qmp(args.args[0], **json.loads(args.args[1] if len(args.args) > 1 else "{}"))
            )
        )
