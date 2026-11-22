"""Send input and capture the two emulated scanouts through this guest's QMP socket."""

import json
import re
import socket
import time


def connect(path, process):
    sock = socket.socket(socket.AF_UNIX)
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError("QEMU exited during startup")
        try:
            sock.connect(str(path))
            return sock
        except (ConnectionRefusedError, FileNotFoundError):
            time.sleep(0.05)
    raise TimeoutError(path)


def serve(work, process):
    with connect(work / "qmp", process) as qmp, connect(work / "commands", process) as commands:
        wire = qmp.makefile("rwb", buffering=0)
        assert "QMP" in json.loads(wire.readline())

        def execute(cmd, **args):
            wire.write(json.dumps(dict(execute=cmd, arguments=args)).encode() + b"\n")
            while True:
                r = json.loads(wire.readline())
                if "event" in r:
                    continue
                if "error" in r:
                    raise RuntimeError(r)
                return r["return"]

        execute("qmp_capabilities")
        con = commands.makefile("rwb", buffering=0)
        for line in con:
            verb, *args = json.loads(line)
            result = True
            if verb == "screen":
                head, label, hexcolor = args
                assert re.fullmatch("[a-z0-9-]+", label) and head in (0, 1)
                image = work / (label + ".ppm")
                execute("screendump", filename=str(image), device="gpu", head=head)
                raw = image.read_bytes()
                header = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", raw)
                w, h = map(int, header.groups())
                rgb = bytes.fromhex(hexcolor)
                pixels = raw[header.end() :]
                x0, y0, x1, y1 = w, h, -1, -1
                for y in range(h):
                    row = pixels[3 * y * w : 3 * (y + 1) * w]
                    first = row.find(rgb)
                    if first < 0:
                        continue
                    last = row.rfind(rgb)
                    while first >= 0 and first % 3:
                        first = row.find(rgb, first + 1)
                    while last >= 0 and last % 3:
                        last = row.rfind(rgb, 0, last + 2)
                    if first >= 0:
                        x0, y0 = min(x0, first // 3), min(y0, y)
                        x1, y1 = max(x1, last // 3), y
                result = dict(size=[w, h], box=[x0, y0, x1 + 1, y1 + 1] if x1 >= 0 else None)
                if label.startswith("gtk-") and result["box"]:
                    scale = (x1 + 1 - x0) / 200
                    offset = 3 * (w * (y0 + round(54 * scale)) + x0 + round(24 * scale))
                    result["stripes"] = list(pixels[offset + 1 : offset + 3 * 16 : 3])
                    # Sample only the "Display 123" glyphs above the stripe target.
                    # Count near-full-contrast edges between adjacent scanout pixels.
                    edges = 0
                    for y in range(y0, y0 + round(32 * scale)):
                        start = 3 * (y * w + x0) + 1
                        row = pixels[start : start + 3 * round(95 * scale) : 3]
                        edges += sum(abs(a - b) > 180 for a, b in zip(row, row[1:]))
                    result["glyph_edges"] = edges
            else:
                if verb == "move":
                    events = [
                        dict(type="abs", data=dict(axis=a, value=v))
                        for a, v in zip(("x", "y"), args)
                    ]
                elif verb == "button":
                    events = [dict(type="btn", data=dict(button=args[0], down=args[1]))]
                elif verb == "key":
                    events = [
                        dict(
                            type="key",
                            data=dict(key=dict(type="qcode", data=args[0]), down=args[1]),
                        )
                    ]
                else:
                    raise ValueError(verb)
                execute("input-send-event", events=events)
            con.write(json.dumps(result).encode() + b"\n")
