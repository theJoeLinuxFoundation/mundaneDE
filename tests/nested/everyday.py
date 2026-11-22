"""Acceptance scenarios inside the headless session created by everyday.sh."""

from pathlib import Path
from dataclasses import dataclass
import os
import signal
import re
import shutil
import subprocess
import sys
import time
from run import wait_for, run, stop

HERE = Path(__file__).resolve().parent


def prepare_ibus(base, host, launch):
    addr = "unix:path=" + str(base / "ibus")
    # This compositor-only probe exercises the input-popup protocol with an external client.
    frontend = base / "test-input-method"
    frontend.write_text('#!/bin/sh\nexec /usr/lib/ibus/ibus-ui-gtk3 --enable-wayland-im "$@"\n')
    frontend.chmod(0o755)
    host.update(IBUS_ADDRESS=addr, LC_ALL="C.UTF-8", MUNDANE_IBUS_PANEL=str(frontend))
    bus = launch(
        [
            "ibus-daemon",
            "--verbose",
            "--address=" + addr,
            "--cache=none",
            "--panel=disable",
            "--config=disable",
            "--emoji-extension=disable",
        ],
        base / "ibus.log",
        host,
    )
    wait_for("isolated IBus daemon", (base / "ibus").is_socket, [bus])


@dataclass
class Session:
    base: Path
    binary: Path
    inner: dict
    host: dict
    launch: object
    capture: object
    background: tuple
    server_pid: int

    def app(self, argv, name, env=None, **kwargs):
        return self.launch(
            argv, self.base / (name + ".log"), dict(env or self.inner, WAYLAND_DEBUG="client"), **kwargs
        )

    def driver(self, *args):
        socket = Path(self.host["XDG_RUNTIME_DIR"]) / self.host["WAYLAND_DISPLAY"]
        run([self.binary.parent / "examples/nested-input", socket, *map(str, args)], self.host)

    def key(self, *codes):
        self.driver(self.base / "keymap", *codes)

    def pointer(self, *events):
        self.driver("pointer", *events)

    def logged(self, name, fragment):
        return fragment in (self.base / (name + ".log")).read_text(errors="replace")

    def controls(self):
        gtk = self.app(
            [sys.executable, str(HERE / "controls.py")], "controls", stdin=subprocess.PIPE, text=True
        )
        wait_for("GTK map", lambda: self.capture("controls") != self.background, [gtk])
        time.sleep(0.5)

        def click_button(name, origin):
            log = self.base / "controls.log"
            offset = log.stat().st_size
            send(gtk, "locate " + name)

            def locate_button():
                with log.open("rb") as stream:
                    stream.seek(offset)
                    return re.search(rf"^button {name} (\d+) (\d+)$", stream.read().decode(), re.M)

            button = wait_for("GTK allocates " + name, locate_button, [gtk])
            geometry = re.findall(r"set_window_geometry\((-?\d+), (-?\d+),", log.read_text())[-1]
            x = origin[0] + int(button[1]) - int(geometry[0])
            y = origin[1] + int(button[2]) - int(geometry[1])
            self.pointer(f"{x},{y}", 272, -272)

        click_button("maximize", (260, 150))
        wait_for("GTK maximize", lambda: self.logged("controls", "configure(800, 450"), [gtk])
        wait_for(
            "maximized GTK covers output",
            lambda: self.capture("maximized")[2][:3] != self.background[2][:3],
            [gtk],
        )
        click_button("maximize", (0, 0))
        wait_for("GTK restore", lambda: self.logged("controls", ".unset_maximized()"), [gtk])
        wait_for(
            "restored GTK bounds",
            lambda: self.capture("restored")[2][:3] == self.background[2][:3],
            [gtk],
        )
        click_button("minimize", (260, 150))
        wait_for(
            "GTK minimize removes window", lambda: self.capture("minimized") == self.background, [gtk]
        )
        self.key(125, 15, -15, -125)
        wait_for(
            "restored minimized GTK window",
            lambda: self.capture("unminimized") != self.background,
            [gtk],
        )
        print("GTK title controls passed", flush=True)
        self.pointer("320,170", 273, -273)
        wait_for(
            "GTK requests the system menu", lambda: self.logged("controls", ".show_window_menu("), [gtk]
        )
        self.key(108, -108, 108, -108, 28, -28)
        wait_for(
            "fullscreen via system menu",
            lambda: self.capture("fullscreen")[2][:3] != self.background[2][:3],
            [gtk],
        )
        send(gtk, "unfullscreen")
        wait_for(
            "leave fullscreen", lambda: self.capture("normal")[2][:3] == self.background[2][:3], [gtk]
        )
        self.pointer("320,170", 272, "300,150", "260,110", -272)
        wait_for(
            "native titlebar move",
            lambda: (
                self.logged("controls", ".move(wl_seat") and self.logged("mundane", "released window at")
            ),
            [gtk],
        )
        # The native resize border straddles the bottom-right edge of window geometry.
        moves = re.findall(
            r"magnetic snap to \(([-0-9]+), ([-0-9]+)\)", (self.base / "mundane.log").read_text()
        )
        x, y = map(int, moves[-1])
        self.pointer(f"{x + 140},{y + 153}", 272, f"{x + 140},{y + 161}", f"{x + 140},{y + 173}", -272)
        wait_for(
            "native edge resize",
            lambda: (
                self.logged("controls", ".resize(wl_seat")
                and re.search(
                    r"configure\(280, (169|170|171),", (self.base / "controls.log").read_text()
                )
            ),
            [gtk],
        )
        print("System menu, fullscreen, titlebar move and edge resize passed", flush=True)
        return gtk

    def activation(self, gtk):
        probe = self.app(
            [
                self.binary.parent / "examples/nested-capabilities",
                Path(self.inner["XDG_RUNTIME_DIR"]) / self.inner["WAYLAND_DISPLAY"],
                self.base / "probe.shm",
            ],
            "tokens",
        )
        wait_for(
            "token issuer has keyboard focus",
            lambda: re.search(r"wl_keyboard[#@]\d+.enter", (self.base / "tokens.log").read_text()),
            [probe],
        )

        def token():
            offset = (self.base / "tokens.log").stat().st_size
            self.key(67, -67)

            def issued():
                with (self.base / "tokens.log").open("rb") as stream:
                    stream.seek(offset)
                    return re.search(rb"^launch token (\S+)", stream.read(), re.M)

            return wait_for("launch token issued for F9", issued, [probe])[1].decode()

        old = token()
        time.sleep(5.2)

        def activate(value, expected):
            offset = (self.base / "controls.log").stat().st_size
            send(gtk, "activate " + value)
            wait_for(
                "GTK presents launch token",
                lambda: self.logged("controls", '.activate("' + value),
                [gtk],
            )
            time.sleep(0.2)
            send(gtk, "focus")
            with (self.base / "controls.log").open("rb") as stream:

                def observed():
                    stream.seek(offset)
                    return ("active " + str(expected)).encode() in stream.read()

                wait_for("activation focus policy", observed, [gtk, probe])

        activate(old, False)
        activate(token(), True)
        stop(probe)
        print("Fresh activation focuses GTK and expired activation preserves focus", flush=True)

    def clipboard_and_restart(self, gtk):
        send(gtk, "copy")
        wait_for("native clipboard ownership", lambda: self.logged("controls", ".set_selection("), [gtk])
        display = run([self.binary, "--x11-display"], self.inner).decode().strip()
        xenv = dict(self.inner, DISPLAY=display)
        xterm = self.app(
            ["xterm", "-fa", "monospace", "-geometry", "40x8", "-e", "sleep", "90"], "xterm", xenv
        )
        wait_for(
            "mapped xterm",
            lambda: (
                subprocess.run(
                    ["xdotool", "getwindowfocus", "getwindowname"], env=xenv, capture_output=True
                ).stdout.strip()
                == b"sleep"
            ),
            [xterm],
        )
        assert run(["xclip", "-selection", "clipboard", "-out"], xenv).decode() == "Wayland café"
        xclip = self.app(
            ["xclip", "-quiet", "-selection", "clipboard"], "xclip", xenv, stdin=subprocess.PIPE
        )
        xclip.stdin.write("X11 日本".encode())
        xclip.stdin.close()
        wait_for("X11 replaces native ownership", lambda: self.logged("controls", ".cancelled()"), [gtk, xclip])
        log = self.base / "controls.log"
        offset = log.stat().st_size
        stop(xterm)
        wait_for(
            "new selection reaches GTK after refocus",
            lambda: b".selection(wl_data_offer" in log.read_bytes()[offset:],
            [gtk],
        )
        send(gtk, "paste")
        wait_for(
            "X11 clipboard paste in GTK",
            lambda: self.logged("controls", "pasted X11 日本"),
            [gtk, xclip],
        )
        send(gtk, "exit")
        assert gtk.wait(timeout=4) == 0
        print("Xwayland clipboard passed", flush=True)
        stop(xclip)
        # Inside the private PID namespace, stop the server whose parent owns the
        # launcher socket. The compositor should release its display and start another.
        pid = self.server_pid
        children = [
            int(p.parent.name)
            for p in Path("/proc").glob("[0-9]*/status")
            if re.search(r"^Name:\s+Xwayland$", p.read_text(), re.M)
            and re.search(rf"^PPid:\s+{pid}$", p.read_text(), re.M)
        ]
        assert len(children) == 1
        os.kill(children[0], signal.SIGTERM)
        x_socket = Path("/tmp/.X11-unix") / ("X" + display[1:])
        wait_for(
            "Xwayland releases its display after the last client exits",
            lambda: not x_socket.exists() and self.logged("mundane", "Xwayland stopped"),
        )
        display = run([self.binary, "--x11-display"], self.inner).decode().strip()
        run(["xdotool", "getdisplaygeometry"], dict(xenv, DISPLAY=display))
        print("Xwayland restart passed", flush=True)

    def terminal_clipboard(self):
        # Both terminals own their own clipboard data (OSC 52) and paste it into a PTY.
        import base64

        from_native = base64.b64encode("from foot café".encode()).decode()
        from_x11 = base64.b64encode("from xterm 日本".encode()).decode()

        def terminal_script(encoded, result, copy_first):
            publish = f"printf '\\033]52;c;{encoded}\\007'"
            read_line = 'IFS= read -r line && printf %s "$line" > "$1"'
            commands = ["read -r ready", publish, read_line] if copy_first else [read_line, publish]
            return ["sh", "-c", " ; ".join([*commands, "sleep 60"]), "shell", str(result)]

        foot = self.app(
            [
                "foot",
                "--maximized",
                "--config=/dev/null",
                *terminal_script(from_native, self.base / "native-paste", True),
            ],
            "foot",
        )
        wait_for(
            "foot keyboard focus",
            lambda: re.search(r"wl_keyboard[#@]\d+\.enter", (self.base / "foot.log").read_text()),
            [foot],
        )
        self.key(28, -28)
        wait_for(
            "native terminal owns clipboard", lambda: self.logged("foot", ".set_selection("), [foot]
        )
        display = run([self.binary, "--x11-display"], self.inner).decode().strip()
        terminal_env = dict(self.inner, DISPLAY=display)
        xterm = self.app(
            [
                "xterm",
                "-bg",
                "#eeeeaa",
                "-geometry",
                "30x6",
                "-fa",
                "monospace",
                "-title",
                "clipboard test",
                "-xrm",
                "XTerm*allowWindowOps:true",
                "-xrm",
                "XTerm*disallowedWindowOps:",
                "-xrm",
                "XTerm*selectToClipboard:true",
                "-e",
                *terminal_script(from_x11, self.base / "x-paste", False),
            ],
            "xterm-paste",
            terminal_env,
        )
        wait_for(
            "X terminal mapped",
            lambda: (
                subprocess.run(
                    ["xdotool", "search", "--onlyvisible", "--name", "clipboard test"],
                    env=terminal_env,
                    capture_output=True,
                ).returncode
                == 0
            ),
            [xterm],
        )
        xid = (
            run(["xdotool", "search", "--onlyvisible", "--name", "clipboard test"], terminal_env)
            .splitlines()[-1]
            .decode()
        )
        bounds = run(["xdotool", "getwindowgeometry", "--shell", xid], terminal_env).decode()
        metrics = dict(line.split("=") for line in bounds.splitlines())
        center = f"{int(metrics['X']) + int(metrics['WIDTH']) // 2},{int(metrics['Y']) + int(metrics['HEIGHT']) // 2}"
        wait_for(
            "X terminal rendered above foot",
            lambda: self.capture("terminals")[2].count(bytes.fromhex("eeeeaa")) > 1000,
            [foot, xterm],
        )
        self.pointer(center, 272, -272)

        def x_focus():
            result = subprocess.run(
                ["xdotool", "getwindowfocus", "getwindowname"], env=terminal_env, capture_output=True
            )
            (self.base / "x-focus").write_bytes(bounds.encode() + result.stdout + result.stderr)
            return result.stdout.strip() == b"clipboard test"

        wait_for("X terminal focus", x_focus, [xterm])
        self.key(42, 110, -110, -42, 28, -28)
        wait_for(
            "foot clipboard pasted into xterm",
            lambda: (
                (self.base / "x-paste").is_file()
                and (self.base / "x-paste").read_text() == "from foot café"
            ),
            [foot, xterm],
        )
        # Keep xterm alive as selection owner while the native terminal receives focus.
        # The smaller X window leaves the top left of foot visible.
        self.pointer("20,40", 272, -272)
        wait_for(
            "foot regains keyboard focus",
            lambda: (
                len(re.findall(r"wl_keyboard[#@]\d+\.enter", (self.base / "foot.log").read_text())) >= 2
            ),
            [foot],
        )
        self.key(29, 42, 47, -47, -42, -29, 28, -28)
        wait_for(
            "xterm clipboard pasted into foot",
            lambda: (
                (self.base / "native-paste").is_file()
                and (self.base / "native-paste").read_text() == "from xterm 日本"
            ),
            [foot, xterm],
        )
        stop(xterm)
        stop(foot)
        print("xterm and foot pasted both clipboard payloads", flush=True)

    def primary_and_drag(self):
        source = self.app([sys.executable, str(HERE / "transfer.py"), "source"], "source")
        wait_for("drag source", lambda: self.logged("source", ".attach(wl_buffer"), [source])
        target = self.app([sys.executable, str(HERE / "transfer.py"), "target"], "target")
        wait_for("drop target", lambda: self.logged("target", ".attach(wl_buffer"), [target])
        time.sleep(0.5)

        def locate(rgb):
            width, _, pixels = self.capture("transfers")
            offset = pixels.find(bytes.fromhex(rgb))
            assert offset >= 0 and offset % 3 == 0
            return (offset // 3 % width + 16, offset // 3 // width + 24)

        sx, sy = locate("12a76d")
        tx, ty = locate("315cef")
        start, finish = f"{sx},{sy}", f"{tx},{ty}"
        self.pointer(start, 272, -272)
        wait_for("primary selection owner", lambda: self.logged("source", ".set_selection("), [source])
        self.pointer(finish, 274, -274)
        wait_for(
            "middle click pastes primary text",
            lambda: self.logged("target", "entry primary text"),
            [target],
        )
        self.pointer(start, 272, f"{sx + 16},{sy}", f"{sx + 24},{sy}", finish, finish, -272)
        wait_for(
            "Wayland drop transfer",
            lambda: self.logged("target", "dropped text") and self.logged("target", ".drop()"),
            [source, target],
        )
        offset = (self.base / "target.log").stat().st_size
        stop(source)

        def source_gone():
            with (self.base / "target.log").open("rb") as stream:
                stream.seek(offset)
                return re.search(
                    rb"zwp_primary_selection_device_v1[#@]\d+\.selection\(nil", stream.read()
                )

        wait_for("selection clears when its owner exits", source_gone, [target])
        stop(target)
        print("Primary selection and drag-and-drop passed", flush=True)

    def input_methods(self):
        engine = self.app([sys.executable, str(HERE / "engine.py")], "engine")
        wait_for(
            "IBus test engine registration", lambda: self.logged("engine", "engine registered"), [engine]
        )
        run([sys.executable, str(HERE / "engine.py"), "--select"], self.inner)
        doc = self.base / "composition.txt"
        doc.write_text("")
        editor = self.app(["gnome-text-editor", "--standalone", str(doc)], "editor")
        wait_for("editor text input", lambda: self.logged("editor", ".enable()"), [editor])

        def editor_candidates(label):
            # Locate the real editor surface and its current caret in output coordinates.
            time.sleep(0.4)
            before = self.capture(label + "-before")
            self.key(36, -36)
            wait_for(
                "IBus candidate surface",
                lambda: self.logged("mundane", ".get_input_popup_surface("),
                [editor, engine],
            )

            def visible():
                geometry = re.findall(
                    r"set_window_geometry\((\d+), (\d+), (\d+), (\d+)\)",
                    (self.base / "editor.log").read_text(),
                )[-1]
                gx, gy, width, height = map(int, geometry)
                origin = re.findall(
                    rf"mapped window {width}x{height}, springing in to \((\d+), (\d+)\)",
                    (self.base / "mundane.log").read_text(),
                )[-1]
                rect = re.findall(
                    r"set_cursor_rectangle\((\d+), (\d+), (\d+), (\d+)\)",
                    (self.base / "editor.log").read_text(),
                )[-1]
                x = int(origin[0]) - gx + int(rect[0])
                y = int(origin[1]) - gy + int(rect[1]) + int(rect[3])
                after = self.capture(label)
                # A blinking caret or preedit underline cannot fill this area below the line.
                delta = 0
                for row in range(y + 5, y + 55):
                    start = 3 * (row * before[0] + x + 4)
                    delta += sum(
                        a != b
                        for a, b in zip(before[2][start : start + 180], after[2][start : start + 180])
                    )
                return (x, y) if delta > 900 else None

            return wait_for("candidate list directly below caret", visible, [editor, engine])

        first_caret = editor_candidates("first-candidates")
        self.key(37, -37)
        wait_for(
            "editor text input commit",
            lambda: self.logged("editor", 'commit_string("日本")'),
            [editor, engine],
        )
        self.key(29, 31, -31, -29)
        wait_for("saved editor composition", lambda: doc.read_text().strip() == "日本", [editor])
        print("Text Editor IBus composition passed", flush=True)
        profile = self.base / "firefox-profile"
        profile.mkdir()
        (profile / "user.js").write_text(
            'user_pref("widget.wayland.text_input_v3.enabled", true);\n'
            'user_pref("browser.shell.checkDefaultBrowser", false);\n'
            'user_pref("browser.aboutwelcome.enabled", false);\n'
            'user_pref("browser.startup.homepage_override.mstone", "ignore");\n'
        )
        page = self.base / "form.html"
        page.write_text(
            '<meta charset="utf-8"><title>Form</title><input autofocus oninput="document.title=this.value">'
        )
        self.key(28, -28, *([50, -50] * 18))
        second_caret = editor_candidates("second-candidates")
        assert second_caret[0] > first_caret[0] + 100 and second_caret[1] > first_caret[1]
        wait_for(
            "second editor preedit",
            lambda: (self.base / "editor.log").read_text().count('preedit_string("に"') >= 2,
            [editor],
        )
        browser = self.app(
            ["firefox", "--no-remote", "--profile", str(profile), page.as_uri()],
            "firefox",
            dict(self.inner, MOZ_ENABLE_WAYLAND="1"),
        )
        wait_for("Firefox form text input", lambda: self.logged("firefox", ".enable()"), [browser])
        wait_for(
            "editor loses input focus mid-composition",
            lambda: self.logged("editor", ".leave(wl_surface"),
            [editor],
        )
        self.key(37, -37)
        wait_for(
            "old preedit cleared in Firefox", lambda: self.logged("firefox", 'set_title("k '), [browser]
        )
        self.key(29, 30, -30, -29, 14, -14)
        self.key(36, -36, 37, -37)
        wait_for(
            "Firefox composition reaches form",
            lambda: self.logged("firefox", 'set_title("日本'),
            [browser, engine],
        )
        print("Firefox IBus composition passed", flush=True)
        stop(browser)
        stop(editor)

    def popup_tree(self):
        menus = self.app([sys.executable, str(HERE / "popups.py")], "popups")

        def click_colour(color):
            rgb = bytes.fromhex(color)

            def position():
                width, _, pixels = self.capture("menu-" + color)
                # Pick the centre of an interior band of the button, away from its outline.
                offset = pixels.find(rgb * 20)
                if offset >= 0:
                    return f"{offset // 3 % width + 12},{offset // 3 // width + 7}"

            pos = wait_for("visible menu control " + color, position, [menus])
            self.pointer(pos, 272, -272)

        for color in ["e4ae17", "aa3876", "1959c4"]:
            click_colour(color)
        wait_for(
            "transient dialog opens from nested menu",
            lambda: (
                self.logged("popups", "opened dialog")
                and self.logged("popups", ".set_parent(xdg_toplevel")
            ),
            [menus],
        )
        assert (self.base / "popups.log").read_text().count(".grab(wl_seat") >= 2
        stop(menus)
        print("Edge menu, grabbed submenu and transient dialog passed", flush=True)

    def rename_in_files(self):
        folder = self.base / "sample-folder"
        folder.mkdir()
        before = folder / "before"
        before.write_text("contents")
        files = self.app(["nautilus", "--select", str(before)], "nautilus")
        wait_for("Nautilus window", lambda: self.logged("nautilus", ".ack_configure("), [files])
        time.sleep(0.6)
        self.key(42, 68, -68, -42)  # Shift+F10 opens the selected file's context menu
        wait_for("Nautilus menu grab", lambda: self.logged("nautilus", ".grab(wl_seat"), [files])
        self.key(1, -1)
        wait_for(
            "Escape closes menu",
            lambda: (
                self.logged("nautilus", "xdg_popup")
                and re.search(r"xdg_popup[#@]\d+\.destroy\(", (self.base / "nautilus.log").read_text())
            ),
            [files],
        )
        self.key(60, -60)  # F2, then replace the selected name
        time.sleep(0.25)
        self.key(30, -30, 33, -33, 20, -20, 18, -18, 19, -19, 28, -28)
        wait_for(
            "rename in Nautilus", lambda: (folder / "after").exists() and not before.exists(), [files]
        )
        print("Nautilus menu, Escape and rename passed", flush=True)
        stop(files)

    def suite_clients(self):
        for name in ["mundane-terminal", "mundane-files"]:
            exe = shutil.which(name, path=self.inner["PATH"])
            assert exe, f"place {name} on PATH before starting the suite"
            icd = next(Path("/usr/share/vulkan/icd.d").glob("lvp_icd*.json"))
            native = self.app(
                [exe],
                name,
                dict(self.inner, VK_DRIVER_FILES=str(icd), WGPU_BACKEND="vulkan", SHELL="/bin/sh"),
            )
            wait_for(
                name + " submits its first buffer",
                lambda: self.logged(name, ".attach(wl_buffer"),
                [native],
            )
            stop(native)
            print(name + " mapped", flush=True)


def send(process, command):
    process.stdin.write(command + "\n")
    process.stdin.flush()


def exercise(base, binary, inner, host, launch, capture, background, server_pid):
    inner.update(
        IBUS_ADDRESS=host["IBUS_ADDRESS"],
        LC_ALL="C.UTF-8",
        GTK_IM_MODULE="wayland",
        GDK_BACKEND="wayland",
        GSK_RENDERER="cairo",
        GTK_A11Y="none",
    )
    (base / "keymap").write_bytes(run(["xkbcli", "compile-keymap", "--layout", "us"], host) + b"\0")
    run(
        [
            binary.parent / "examples/nested-capabilities",
            Path(inner["XDG_RUNTIME_DIR"]) / inner["WAYLAND_DISPLAY"],
        ],
        inner,
    )
    session = Session(base, binary, inner, host, launch, capture, background, server_pid)
    gtk = session.controls()
    session.activation(gtk)
    session.clipboard_and_restart(gtk)
    session.terminal_clipboard()
    session.primary_and_drag()
    session.input_methods()
    session.popup_tree()
    session.rename_in_files()
    session.suite_clients()
