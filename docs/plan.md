# Replacing GNOME 51 on x86 joe Linux

The first distribution deliverable is a second session in GDM. A default
change comes after a fresh installation and an upgraded installation both pass
the final milestone. Application replacement has a separate schedule. A working
desktop must be able to run today's Nautilus, Text Editor and Firefox while the
GPUI suite matures. [Public roadmap](../ROADMAP.md) · [joe desktop package](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/joe-desktop/package.zon)

## Starting point and responsibilities

At `8fc7b43`, both backends share a Smithay `Space`, one output, a GLES
renderer, spring placement and a wallpaper engine. The compositor registers
xdg-shell, shared memory, seat, output and data-device globals. A toplevel
receives an initial configure and maps on its first buffer. Popup creation,
grabs and repositioning are empty handlers. Output geometry assumes scale one.
DRM selects one connector, and session pause currently suspends libinput. Both
loops wake on a 16 ms timeout. The GPUI panel writes compositor TOML and talks
to system services. Its Wayland display backend is a stub. [Shell](../src/shell/mod.rs) ·
[State](../src/state.rs) · [DRM backend](../src/backend/drm.rs) · [Settings panel](../settings-panel/README.md)

The target is x86/glibc with systemd 262 and logind, Mesa 26.2, GTK 4.24,
libadwaita 1.10, Xwayland 24.1, xdg-desktop-portal 1.22 and PipeWire 1.6
with WirePlumber. GNOME Shell currently supplies the panel and launcher via
Dash to Panel and ArcMenu. Keep GDM, the first-account flow, keyring, GVfs,
fonts, icons and MIME handlers available during the transition. The distribution
maintainer owns package recipes, neomake VM integration and the eventual
default. This repository supplies the compositor, shell, settings panel,
session files and reproducible test scenarios. [Distribution package list](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/joe-desktop/package.zon)

## 1. Establish a repeatable compositor test

Run the locked compositor and panel workspaces on stable Rust. Keep Smithay and
smithay-drm-extras together at v0.7.0 (`a166cf4`) for the baseline. At a
milestone that needs an upstream fix or protocol, identify the missing API,
prefer a released tag and pin an exact commit if an unreleased fix is necessary.
Update both dependencies, compatible calloop/xkbcommon versions, the lockfile
and the pin record together. Review protocol version changes, redo the registry
and screenshot checks, and exercise DRM pause/resume before accepting the
upgrade. Keep the previous lockfile recoverable with a normal revert. [Smithay pin](../PINNING.md)

**Nested EGL finding.** On the NVIDIA Wayland host, a cold start logged
`EGL_BAD_SURFACE` from `eglQuerySurface` inside Smithay's `buffer_age` trace
span. Our render function queries age before the first `backend.bind()`.
Smithay documents that the query needs a bound surface. The existing error
fallback gives damage tracking age zero, so rendering proceeds. An isolated
build that skipped the initial query and used age zero drew Afterlight and
mapped `foot` without the error. For the implementation, use full damage until
a successful first frame, invalidate age on surface recreation, and retain age
tracking on subsequent valid frames. [Nested render path](../src/backend/winit.rs) ·
[Smithay buffer-age contract](https://github.com/Smithay/smithay/blob/a166cf4c94b5aedc332a65aa1dd753e8148829c3/src/backend/winit/mod.rs#L305-L320)

**Acceptance.** From private XDG directories and a private session bus, start
winit inside a headless Wayland host with Mesa software rendering. Wait for
socket readiness, map a real xdg-shell fixture with a known colour, resize it,
assert the configure/ack and new buffer dimensions, then close it. Capture the
host output before mapping, after resize and after close. Require the expected
pixels and a clean child exit. Repeat cold starts and resize on the affected
driver with buffer-age tracing. Check for zero startup EGL errors and correct
damage on later frames. Record the baseline registry for an ordinary client. Use this run for the
first Terminal integration with the app suite, including its keyboard-focus
and screen-reader probe (milestone 7).

## 2. Make everyday clients work

Treat protocol support as behaviour a client can exercise. Record the global
name and negotiated version alongside the client, backend and test result.
Use the upstream XML and the version shipped with the client when implementing
each row. [Wayland protocol definitions](https://gitlab.freedesktop.org/wayland/wayland-protocols) ·
[wlr protocol definitions](https://gitlab.freedesktop.org/wlroots/wlr-protocols)

| Interface | Behaviour and protocol test |
| --- | --- |
| xdg-shell and xdg-decoration | Handle popup trees, serial-checked grabs, reposition/dismiss, interactive move/resize, maximized and fullscreen configure/ack. Negotiate client-side decorations for GTK and a usable decoration mode for other clients. Exercise an edge-positioned menu, a nested submenu and a transient dialog in Nautilus. |
| wl_data_device, primary-selection and data-control | Update selection on focus changes. Test clipboard, drag-and-drop and middle-click selection across two clients, including source exit. Offer ext-data-control only to an authorized clipboard manager, with wlr-data-control compatibility if the selected manager needs it. |
| text-input and input-method | Connect text-input-v3 clients to a trusted input-method-v2 frontend backed by IBus. Forward preedit, commit, surrounding-text changes, focus and caret geometry. On joe, provide a session frontend talking to the packaged IBus daemon and a candidate popup. The joe recipe disables the upstream Wayland frontend and GTK input modules. Test with a fixture engine that produces candidates and complex text. [joe IBus build](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/packages/ibus/build.sh) |
| cursor-shape and xdg-activation | Honour cursor changes over text, links and resize edges. Use recent seat serials and activation tokens for launches and notification actions. A valid user launch focuses its window, while a stale request leaves focus where the user put it. |
| Xwayland and xwayland-shell | Start rootless Xwayland on demand, associate X11 windows with Wayland surfaces through Smithay's XWM, and bridge focus, transient windows, clipboard ownership and close requests. Exercise native and X11 clients side by side and restart Xwayland after closing the X clients. |

**Acceptance.** Open a folder and rename a file in Nautilus, dismiss its menus
with Escape, and type immediately in the same window. Scope the prototype's
global Escape-to-quit shortcut to development use. Compose and commit the same
string in GTK Text Editor and a Firefox form, switch focus mid-preedit and check
the candidate popup follows the caret. Copy in both directions between
`xterm` and a native terminal. Run the suite's Terminal and Files as additional
native clients when their first runnable revisions arrive.

## 3. Drive each display at its own scale and refresh rate

Replace the single output and flip flag with per-connector mode, logical
position, transform, scale, damage history and presentation clock. Map input
in logical coordinates and choose an output for each surface and its popups.
On unplug, move windows, focus and panels to an available display. Retain a
known-good layout and require confirmation with a timeout for mode changes.

| Interface | Behaviour and test |
| --- | --- |
| fractional-scale-v1 and viewporter | Send preferred fractional scale, apply viewport source/destination correctly and retain integer-scale fallback. Move GTK, Firefox and GPUI between 100%, 150% and 200%. Check a sharp text sample, exact pointer hit targets and a popup crossing the boundary. |
| linux-dmabuf and presentation-time | Advertise formats/modifiers the renderer imports, send feedback and retain the shared-memory path. Exercise a Mesa/EGL client and a GPUI wgpu client, compare colours, and check presented/discarded feedback against DRM page flips. Record the timing source on nested runs. |
| tearing-control-v1 | Treat the requested presentation mode as a hint. Permit asynchronous flips only on a capable output under the chosen fullscreen policy. Verify normal windows remain synchronized and an unsupported request falls back to vsync. |

**Acceptance.** With two outputs of different scale and refresh, drag a window
between them, unplug its current output, reconnect, then reject a proposed mode
and recover the previous layout. Check window bounds, cursor, focus and content
in each state. Use virtual outputs for geometry and real DRM hardware for
hotplug, modesetting, modifier import and scanout timing.

Magnetic placement should go unnoticed during an ordinary drag. Verify the
window follows the pointer until release, settles at a nearby edge or neighbour
and stays put when released outside the attraction radius. Repeat with reduced
motion enabled and use the same target immediately, without overshoot or a fade.

**Spring and frame budgets.** Proposed targets are measured in a release DRM
build on a recorded reference GPU/CPU, first at 1920×1080/60 Hz and then at
120 Hz on a capable display. Script ten moving windows and repeat drag, snap
and wallpaper fade for 60 seconds after warm-up. Record p50/p95/p99 spring
CPU time, composition time and presentation intervals per output. Target p95
spring update below 1 ms, p95 CPU update plus render submission below 8 ms at
60 Hz and 4 ms at 120 Hz, and fewer than 1% missed refresh deadlines in each
run. Drive frames from damage, active springs and pending client work. In a
10-second settled scene with a stationary cursor and no client updates, expect
zero animation redraws. In a deterministic clock test, drop a frame, check
bounded motion and exact final positions, then verify the final settled frame
reaches the screen. Keep software-rendered screenshot runs separate from timing
measurements.

## 4. Enter and leave a real session

Supply a GDM entry under `/usr/share/wayland-sessions/` and a session launcher
that keeps the compositor in the active logind seat. Use libseat's logind
backend. Hand the distribution the libseat build requirement. Pause DRM/KMS
and input on VT loss, then reacquire devices and refresh outputs on return.
The compositor, shell and settings run as the logged-in user.

Once the socket is listening, publish `WAYLAND_DISPLAY`,
`XDG_CURRENT_DESKTOP=mundane` and the Wayland session type to D-Bus activation
and the systemd user manager. Start a mundane session target bound to
`graphical-session.target`. Order the shell, polkit agent, IBus frontend and
portal activation after readiness. Start X11 clients with the session's
Xwayland display. On exit, stop session-bound units, release the seat and
clear stale session values. Keep nested tests on a private bus. Privileged
operations go through existing system services and polkit. Supply the
distribution with the operation list for any new privileged helper. [systemd session targets](https://www.freedesktop.org/software/systemd/man/latest/systemd.special.html) ·
[joe security model](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/README.md#L174-L215)

**Acceptance.** In an opt-in GDM session, verify the active seat and user IDs,
open a D-Bus-activated app on the correct socket, switch to another VT and
back, suspend/resume, log out and log in twice. Verify input and scanout return,
the old socket is gone, no duplicate units survive and GNOME still starts from
GDM. Repeat a protected settings action with authorization granted and denied
on the distribution's enforcing configuration.

## 5. Provide a shell, workspaces and a lock screen

Preserve the bottom taskbar, Start menu, StatusNotifierItem tray and desktop
icons. Show the user’s XDG Desktop directory, support keyboard selection and
open items through their preferred handler. Provide minimize and maximize
buttons on normal resizable windows. Agree decoration mode with each toolkit,
configure GTK header bars and implement the controls on server-decorated
windows. Test both kinds. Minimize to the taskbar, restore from it, maximize
up to the taskbar and restore the original bounds. Launch from Start and from
a desktop icon, and activate a tray item. Compare the default layout against
the joe GNOME session at the same resolution. [Desktop announcement](https://joelinux.io/news/mundanede/)

Deliver the panel, launcher, notification service, on-screen displays (OSDs)
and system menu as restartable session components. Read installed desktop
entries, match Wayland app IDs, honour MIME/URI handlers and use activation
tokens. Provide a task list, workspace switching, a keyboard-searchable
overview, notification actions/history and a visible sharing indicator with a
stop control. Support StatusNotifierItem tray applications used on joe.
The compositor owns placement and workspace membership across shell restarts.

| Interface | Consumer and acceptance test |
| --- | --- |
| wlr-layer-shell | The panel reserves its exclusive zone on each output. Test a maximized window beneath it, a keyboard-interactive launcher, and a transient volume OSD that leaves typing focus with the app. |
| foreign-toplevel | Use ext-foreign-toplevel-list for enumeration. Give the trusted shell a control path for activate, close and workspace moves. Listing alone cannot perform those actions. Match two windows of one app, move one and recover the correct focus after closing it. |
| idle-inhibit, idle-notify and ext-session-lock | Play a video with a visible inhibitor, hide it and verify idle policy resumes. Explicit lock always wins. Cover every output before reporting locked, route input only to the locker, and cover newly connected outputs immediately. Keep a solid locked surface if the locker disappears. Authenticate unlock through PAM. |
| security-context-v1 | Track the sandbox context associated with new connections. Filter capture, input-method, data-control, lock and shell-management globals by the authorized connection. Establish trusted session components through compositor-issued connections. Treat self-reported app IDs as display metadata. Compare ordinary, sandboxed and authorized registries and operations. |

Connect the system menu to NetworkManager for network state and connections,
PipeWire/WirePlumber for devices, default sink and volume, UPower for battery
status, logind for suspend/power actions, and BlueZ for pairing and connection.
Mirror device changes made elsewhere. Use a polkit agent when the service asks
for authorization. Handle a missing radio, disappearing sink and cancelled
pairing as visible, recoverable states.

**Acceptance.** With two workspaces, launch and find an app by keyboard, move
its window, activate a notification and recover the previous focus on dismiss.
Toggle Wi-Fi, change audio output, show its volume OSD, pair a test device and
read battery status. Restart the panel and retain windows/workspaces. Lock two
displays, attach a third, terminate the locker, and verify coverage and
input isolation until a recovered locker authenticates. Include a suspend/lid
cycle with the session locked before sleep. Exercise service failures on a
private test bus and repeat device actions on a laptop.

### Upgrade and recovery status on the desktop

The shell owns a persistent entry in the system menu and a notification when
an upgrade is prepared or a failed upgrade has returned the machine to its
previous system. Show these at login and refresh during the session, including
when Packages is closed. Offer a normal, user-confirmed restart for a prepared
upgrade. Keep a recovery notice available after its notification is dismissed.

Request a versioned, unprivileged, machine-readable snapshot from dave. The
shell and Packages should share the following meanings. The exact transport
and command spelling belong to dave. [dave(8)](https://github.com/theJoeLinuxFoundation/joe-linux/blob/beta2/dave/share/dave.8)

| dave operation | Data the desktop needs |
| --- | --- |
| `deployments list` and `status` | Stable IDs and display labels for bootable deployments, the running and next-boot IDs, and the durable prepared upgrade ID/target or an explicit empty value. A prepared upgrade is applied during shutdown. The next-boot ID alone does not establish preparation. |
| `upgrade --check` | A read-only preview of available changes with package names, actions and old/new versions, an empty result for up-to-date, and a distinct failure result. A nonempty preview alone must never trigger the restart notice. |
| Boot outcome | A durable last-upgrade event ID, attempted deployment/upgrade ID, actual running ID, outcome (succeeded, automatic fallback after failure, explicit rollback, or unknown) and a displayable reason when known. A manual rollback or a user selecting an older deployment must not produce a failed-upgrade alert. |

Read a consistent snapshot at login, after resume and when dave signals a state
change (a revision to poll is sufficient). Deduplicate notifications by upgrade
or event ID across shell restarts. Request fixtures for preview only, prepared,
successful boot, automatic fallback, manual rollback and unavailable/unsupported
status. Test these without elevated privileges. Only prepared shows “Upgrade
ready. Restart to apply”. Automatic fallback names the attempted and restored
systems. Successful boot clears the pending notice. Treat unreadable or newer
unsupported data as unavailable. Never infer a boot failure by comparing IDs or
parsing translated CLI output.

## 6. Deliver portals that work outside GNOME Shell

Use the distribution's xdg-desktop-portal frontend with
`mundane-portals.conf` selected by `XDG_CURRENT_DESKTOP=mundane`. Start with
the GTK backend for FileChooser, AppChooser, Settings and Notification, and
retain the keyring Secret backend where available. Test the frontend's OpenURI
flow with the user's preferred handler. Route Screenshot and ScreenCast to
xdg-desktop-portal-wlr. The distribution will need a recipe for that backend
and its selected chooser/screenshot helpers. It is a Wayland client and can
run against Smithay when the negotiated capture protocols are implemented.
[Portal routing](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html) ·
[wlr backend interfaces and setup](https://github.com/emersion/xdg-desktop-portal-wlr#running)

Implement **wlr-screencopy** for output capture first. Add
**ext-image-copy-capture-v1** with the matching **ext-image-capture-source-v1**
output source and, for individual window sharing, a foreign-toplevel source.
Record the backend revision and the interfaces it actually binds. Offer only
sources that the negotiated path can capture. Authorize the portal backend
and any capture helper it launches through scoped connections. Ordinary apps
request capture through the portal. Require a visible source chooser and
consent before returning an image or PipeWire stream. If the packaged backend
cannot enforce consent or delegate capture safely to a helper, implement that
portion in a mundane backend before enabling it in the session. [Capture protocols used upstream](https://github.com/emersion/xdg-desktop-portal-wlr/blob/master/protocols/meson.build) ·
[ScreenCast API](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)

**Acceptance.** On a private bus, start a Firefox WebRTC share with two outputs
showing different test patterns. Select one, verify the received PipeWire
frames contain only that output and see the sharing indicator. Stop from the
shell, cancel a second request, then test source unplug, browser exit and session
lock during a share. Verify stream teardown and no lock-screen pixels. Request
a screenshot, check the returned URI and its pixels, then deny a fresh request
and check that no image is delivered. Repeat source isolation for a single
window before advertising window sharing. From Files or a suite fixture, open
and cancel a file chooser, open a URI in its handler, and activate a
notification. Check the active backend names in the session journal.

## 7. Share settings, appearance and accessibility

Keep the MIT-licensed GPUI Settings application in this repository.
`mundane-settings-core` owns validation, migrations and the system adapters.
Keep spring, gaps and wallpaper in the compositor's XDG TOML file. Preserve
comments and prior keys, write atomically, validate numbers before applying,
and retain working settings after an invalid edit. Give Settings an
authenticated output-management path with test/apply/revert for display
layouts. Replace the Wayland display stub and keep NixOS upgrade commands out
of the joe workflow. Direct package management to the suite's Packages app
when its dave integration is ready. Use AccountsService, timedated and the
same device services as the system menu for the relevant pages.

**Agreement with mundane-apps.** At the suite's Terminal milestone, agree on
semantic foreground/background, selection, error, accent and focus roles,
spacing, fonts, keyboard actions and a sample settings row with a labelled
control. Probe AT-SPI on that control immediately. Once the suite publishes
the crate and the sample, pin `mundane-ui` by full git revision in the panel
workspace and commit the lockfile. Keep GPUI and gpui_platform on Zed
`deb48ceb47125223df73902d1c3b72f2f442222b` in both repositories. Check
Cargo's resolved graph for a single GPUI source and run the shared sample on
Wayland on each pin update. The suite owns widgets and `mundane-services`
client adapters. Settings owns its data model and operations. At the Files
milestone exchange desktop IDs, MIME inputs and success/cancel/error fixtures
for the M6 portal tests. [Application suite plan](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/main/docs/plan.md)

Use desktop GSettings preferences for light/dark, high contrast, accent,
font/text scale and animation preference where supported. GTK/libadwaita use
their native styles. Map the same preferences to semantic roles in
`mundane-ui` and the shell. Read standard appearance keys from the Settings
portal and subscribe to changes. Verify which keys joe's GTK backend exports.
Fill any missing required keys in the desktop Settings backend using the same
preference source. Resolve system fonts through fontconfig and icons through
the installed icon theme. When reduced motion is enabled, settle all active
springs to their targets, skip zoom/slide and finish any crossfade in the next
frame. [Portal appearance settings](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html)

Build in keyboard traversal, visible focus, labelled controls, roles, values
and accessible actions for the launcher, notifications, menus, Settings and
locker. Expose the shell and GPUI accessibility tree on AT-SPI and check GTK's
existing tree alongside it. If the pinned GPUI Linux platform lacks the bridge
for a field or dialog, deliver it with the suite and repeat the Orca test.
Provide a keyboard path to enable accessibility at login and retain it after
lock and logout.

**Acceptance.** Open a GTK/libadwaita app, Settings and a GPUI suite app side
by side. Change light/dark, accent, high contrast, text scale and reduced
motion. Check live updates, legibility, focus rings and lack of clipping at
100%, 150% and 200%. Toggle reduced motion mid-snap and mid-fade. With the
pointer unused, launch an app, adjust a setting, dismiss a dialog and unlock.
Orca must announce the focused role, name, value and state changes, with
passwords masked. Inspect the AT-SPI tree and activate the control through
its accessible action. Restart the session and check persistence.

## 8. Qualify the default session

Provide a distribution handoff with pinned sources, locked offline build
inputs, native dependencies, session entry and user units, portal routing,
desktop IDs, helper operations, release build flags and expected test results.
Record the compositor, suite, package image, Mesa, kernel, GPU/driver and
display modes for every release candidate. Provide a documented GNOME session
fallback during rollout. Hand joe's maintainer the scenarios and expected
artifacts for neomake. Its image build, recipe changes and default switch are
distribution work. [joe Linux test infrastructure](https://github.com/theJoeLinuxFoundation/joe-linux/blob/d3e355b888ebeb5d9fa2c46eace844aafdf9380b/README.md#building-and-testing)

**Acceptance.** Boot clean and upgraded x86 images. Create the first account,
log in through GDM, open a file from a removable drive in its preferred app,
edit and save it, use Firefox with sound and screen sharing, run an X11 app,
lock, suspend, resume and log out. Repeat the protected operation checks under
joe's enforcing security module. Run joe’s existing `make test-desktop` application matrix on the candidate session
in both logging and enforcing modes. In disposable images, stage an upgrade,
see the pending notice, reboot successfully and see it clear. Exercise joe’s
failed-boot recovery fixture and confirm the next login names the restored
system without opening Packages. Check an explicit rollback separately.
Test one multi-output hardware configuration
and a VM. Confirm selecting GNOME at the greeter restores a working session
with the same files and preferences. Publish pass/fail results and unresolved
limitations before recommending a default change. Retire individual GNOME apps
only after the suite's corresponding workflow passes on the image.

## How to collect test evidence

| Where | Evidence to keep |
| --- | --- |
| Rust and service tests | Run `cargo test --locked` in the root and `cargo test --locked --workspace` in `settings-panel/`. Extend the current spring, snap, wallpaper, parser and TOML tests with clock-driven final frames, output transforms, focus transitions and config migration. Use private D-Bus fixtures for disappearing devices, cancellation and denied authorization. |
| Nested and headless tests | Start a disposable host compositor, private runtime/config/data directories and a session bus. Wait for socket and service readiness with a timeout. Drive real Wayland processes with input delivered by the harness. Record registry, configure/ack, focus and client exit. Keep virtual input confined to the test session. Supplement these tests with GPUI headless widget tests. |
| Screenshot checks | Pin the Mesa software renderer, fonts, locale, fixture data, geometry, scale, wallpaper, cursor and clock. Capture at a named state or spring tick, including the frame after settling. Compare geometry exactly and use a documented per-channel tolerance for pixels. On failure save reference, actual, diff and logs. Review changed reference images with the feature that changed them. |
| DRM, VM and hardware | Use the existing manual QEMU path for KMS smoke tests. Add automated joe image scenarios through the distribution handoff. Verify VT ownership, mixed-refresh presentation, hotplug, lock and suspend on real devices. Store timing traces alongside hardware details. [Manual QEMU test](../tests/qemu/README.md) |
