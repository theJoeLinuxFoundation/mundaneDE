# Maintainer’s real-machine session test

Run this on a real x86 machine with the candidate selected in GDM, after automated
qualification passes on clean and upgraded joe Linux images. Use a recoverable
test installation for the boot recovery cases. Extend the checklist when a
milestone lands. Each row describes the expected release behaviour.

For an earlier desktop trial, run `./tests/try-mundane.sh` in a clean clone.
It fetches the published references from `origin` and builds an export of
`origin/main` in the cache. The terminal and the QEMU window title show the
full commit ID. Use `--revision COMMIT` to repeat a build. Both the checkout
commit and the selected revision must be reachable from a published origin
branch or tag. A dirty checkout or an unpublished commit requires
`--from-working-tree`, which labels the build as a working tree.

The visible guest keeps its disk in `~/.cache/mundane-try/desktop`. Headless
qualification defaults to `~/.cache/mundane-try/qualification`. The guest
opens at 1920×1080. Log in or unlock as `joe` with password `joe`. Quit with
**Quick Settings → Shut Down**.
[Guest setup and controls](../tests/qemu/session/README.md#try-the-desktop)

Record the date, tester, candidate image ID, compositor and suite revisions,
machine, GPU/driver, attached displays, scales and input devices. Link the
automated qualification results. For each row, record **pass**, **fail** or
**blocked** with the observation and any issue or retest reference.

| Area | What to try | What good looks like | Result |
| --- | --- | --- | --- |
| Login and layout | Log in through GDM. Compare the layout with joe’s GNOME session at the same resolution. Open an app from Start. Press Super+D, select a desktop file with the arrow keys and open it with Enter. Activate a tray icon. | The taskbar is along the bottom. Start, tray and desktop icons are easy to find and each action opens the expected item. | |
| Window controls | Click a GTK window’s own minimize and maximize buttons. Restore it from the taskbar, maximize and restore again. Repeat with a GPUI app and a server-decorated window. | Each window has one set of controls. Minimizing hides it, the taskbar restores it, maximizing leaves the taskbar usable and restoring recovers its previous bounds. | |
| Placement and motion | Drag slowly near an edge and a neighbouring window, then release far from either. Toggle reduced motion during a snap and repeat. | The window tracks the pointer. A nearby release settles quietly into alignment, a distant one stays put. Reduced motion reaches the same target immediately without bounce or fade. | |
| Files and menus | In Files and Nautilus, browse a folder, open a file from removable media, edit and save it. Open a menu near a screen edge, use a submenu, dismiss with Escape, then rename a file. | Menus stay reachable, Escape returns focus, the correct handler opens and the saved content and new name survive reopening. | |
| Terminals and clipboard | Open the suite’s Terminal. Copy non-ASCII text both ways between a native terminal and xterm. Try middle-click paste and drag a text selection between two accepting apps. | Text arrives intact once in the intended window. Middle click pastes the primary selection. Closing the source leaves the destination responsive. | |
| Input methods | Use an installed IBus engine in Text Editor and a Firefox form. Compose at two caret positions, choose a candidate, then switch apps mid-preedit. | Preedit is visible, candidates follow the caret and the chosen text commits once. The next field starts clean. | |
| Firefox display share | Play audio. On a WebRTC call, cancel a request, then select one of two displays showing different content. Stop from the taskbar. Repeat and unplug the shared display, quit Firefox, or lock during the call. | Audio reaches the chosen output. The peer sees only the selected display after consent. The taskbar names the share. Each stop removes the indicator and ends the stream. The peer never sees the lock screen. | |
| Single-window share | Share a window, move another app over it, then close the selected window. | The peer continues to see only the selected window until it closes. The stream and indicator then end. | |
| Screenshot | Request an image of a display, then of a covered window. Inspect both PNGs. Deny a fresh request. | Each image has the chosen content, colours and orientation. Cancellation returns no image. | |
| GTK portals | Select and cancel a file in a portal file chooser, open a URI with a preferred handler, and activate a notification button. | The caller receives the selected file, cancellation leaves its selection alone, the handler receives the URI and the notification action reaches the originating app. | |
| Displays | Drag GTK, Firefox and Files between displays at 100%, 150% and 200%. Click a small target and open a menu at the outer edge. Unplug the focused display and reconnect it. | Text and hit targets remain aligned, menus stay reachable and the window, pointer and keyboard remain usable on the surviving display. | |
| Display recovery | Try an advertised mode and rotation. Let the 15-second trial expire. Then try a layout with a display to the left of the primary, confirm it and reconnect that display. | The first trial restores the old resolution, rotation and pointer access. The confirmed layout survives reconnect and login. Moving left across the boundary reaches the expected window. | |
| Shell and settings | Switch workspaces, activate a notification, change volume and audio output, toggle Wi-Fi and connect a Bluetooth device. Restart the panel. | Focus follows the chosen window, the OSD leaves typing focus alone, device state matches the services and panel restart preserves windows and workspaces. | |
| Start, overview and panel recovery | Press Super+Space, type an installed app’s name and press Enter. Open two windows of that app. Find one by title in Window overview, use Move →, switch with Super+1–4 and restore a minimized window. Restart the panel in the test session. | Search opens the chosen app. Only the selected window moves. The panel returns with the same workspaces and tasks and can activate and close each one. | |
| Tray, history and volume | Activate a tray app and an action on a notification. Open the clock’s system menu, inspect history and dismiss an entry. While typing in an editor, press a volume key and keep typing. | The intended app receives each action once, dismissal removes the entry, and the volume overlay leaves the editor’s text and keyboard focus intact. | |
| Laptop controls | Switch between two audio outputs, unplug the selected one and toggle Wi-Fi. Pair a Bluetooth device, first cancelling and then confirming its passkey. Check battery percentage and reconnect the device. | The selected sink and radio agree with the system services. Missing hardware and cancellation leave the menu usable. The battery reading and Bluetooth connection match the machine. | |
| Lock and input isolation | With video inhibiting idle, press Super+L. Enter an incorrect password and type an identifiable string. Disconnect the locker on the recoverable test machine, attach a display, close and reopen the lid and enter the correct password. | No application pixels appear on any locked output. Only the locker sees the password and test string. The recovered prompt authenticates through PAM and the previous app accepts input again. | |
| Appearance and accessibility | Change contrast, text size, appearance and reduced motion with GTK and GPUI apps open. Use only the keyboard and Orca to launch an app, change a setting and dismiss a dialog. | Both toolkits update live, all text fits and focus stays visible. Orca announces the focused control’s name, role, value and changes. Preferences persist after login. | |
| Reader at login | Press Super+Alt+S, open Start with Super+Space and type an app name. Open Settings with Super+comma. Tab through Appearance, activate High contrast with Space and log back in. | Orca reads the launcher search, focused toggle and state. The focus ring is visible and the reader and preference survive login. | |
| Shell authentication | Request a timezone change. Read Details and the offered administrator identities. Cancel, try again, lock during entry and unlock. Make a fresh request and authenticate. | Cancellation and lock leave the timezone unchanged. Orca identifies the masked entry. The password stays private, the fresh change succeeds and Settings regains focus. | |
| Live appearance | Open a libadwaita window and the suite Terminal beside Settings. Change light/dark, accent and high contrast in Settings, then change text size. Compare the Terminal grid and cursor, native controls and taskbar. Repeat at 100%, 150% and 200% display scale. Enable Reduce motion during a snap or wallpaper change. | Each open window updates, text fits, controls remain reachable and the motion settles on the next frame. | |
| Orca character echo | Enable character echo. In the GTK test entry, type `z` after `Tokyo`. Start a fresh trial and let the app append `z` to the focused entry without touching the keyboard. Repeat in the GPUI entry. | Both trials show `Tokyoz`. In each toolkit, Orca speaks “z” once for the typed character and stays silent about the application-inserted character. | |
| GDM session lifecycle | Select mundaneDE in GDM. Open Settings and cancel a protected timezone change, then repeat and authenticate. Type in a window before and after a VT switch and suspend. Log out twice, open an app each time, then select GNOME. | Cancel keeps the timezone and authentication applies it. Each login has working input and freshly drawn windows. Each logout returns to GDM. GNOME opens normally. | |
| Seat, lock and power | Switch VTs and back. Lock two displays, attach another and exercise locker loss/recovery. Suspend while locked, resume, authenticate and log out. Log in twice. | Every display stays covered through locker loss and resume. Only the locker receives input until authentication. Input, display and app launching work on each new login. | |
| Upgrade ready | With Packages closed, check for an available upgrade, then prepare it. Dismiss the notification, restart the shell and prepare the same packages again. Try a different upgrade with the same counts, inspect the system menu and complete a successful restart. | A preview alone shows no restart prompt. “Upgrade ready, restart to apply” appears once for each distinct prepared upgrade. Opening Restart offers a confirmation that can be cancelled. The system menu retains the status and it clears after a successful boot. | |
| Upgrade recovery | On the recoverable image, run joe’s failed-upgrade boot fixture and log in to the restored system. Dismiss the alert, reopen the system menu and restart the shell. Separately exercise an explicit rollback. | The desktop identifies the failed target and the previous system now running. Details remain available without duplicate alerts. An explicit rollback produces no failed-upgrade alert. | |
| Session fallback | Select GNOME at the greeter, open the saved file, then log back in to mundaneDE. | Both sessions start, the same file is accessible and the new session has working input and app launch. | |

## Retest the first hand-test findings

The maintainer found these three issues on `5332dbc`. Repeat the checks in
the same app you used for the first trial. Record your observed result and
candidate commit in the last column. The automated retest passed on a headless
joe beta1 guest for all three rows. [Before and after at 1920×1080](screenshots/shell/README.md)

| Finding on `5332dbc` | What to try on the candidate | Automated coverage | Maintainer retest |
| --- | --- | --- | --- |
| An open menu stayed open on a second click. | Focus an editor. For Start, the clock, Workspaces and a tray item with a menu, open it and click its button again. Repeat with Escape and then with a click on an editor control. Type after each dismissal. | The guest checks that each menu closes, the outside click leaves the control untouched and typing reaches the editor again. | Pending |
| The system menu made device controls and power actions hard to pick out. | Open Quick Settings with the clock and with Super+S. Read and toggle Wi-Fi, Bluetooth and Do Not Disturb. Adjust volume and laptop brightness with the keyboard. Compare battery state with the machine. Hover or focus each power icon, then open and cancel Restart and Shut Down. Repeat in high contrast and on a smaller display. | The guest exercises radio state, notification history, the volume slider and both confirmations. All power buttons fit at 960×540 logical pixels in high contrast. A hardware fixture checks optional battery, backlight and Bluetooth controls. | Pending |
| The workspace selector needed clearer state, feedback and navigation. | Put a window on Desk 2 and leave Desk 3 empty. Hover the workspace markers, open the selector, switch with the mouse, wheel and keyboard, then repeat with reduced motion. | The guest checks current and occupied states, input paths and intermediate scanout frames. Reduced motion switches directly to the destination. | Pending |

Record fixes and repeat affected checks on the candidate to be signed off.
A failed or blocked row keeps the gate open. The distribution may change the
default only after the maintainer records their decision for that candidate.

| Candidate image and revision | Automated results | Completed checklist and retests | Maintainer sign-off and date |
| --- | --- | --- | --- |
| | | | |
