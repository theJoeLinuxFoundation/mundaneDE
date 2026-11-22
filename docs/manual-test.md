# Maintainer’s real-machine session test

Run this on a real x86 machine with the candidate selected in GDM, after automated
qualification passes on clean and upgraded joe Linux images. Use a recoverable
test installation for the boot recovery cases. Extend the checklist when a
milestone lands. Each row describes the expected release behaviour.

Record the date, tester, candidate image ID, compositor and suite revisions,
machine, GPU/driver, attached displays, scales and input devices. Link the
automated qualification results. For each row, record **pass**, **fail** or
**blocked** with the observation and any issue or retest reference.

| Area | What to try | What good looks like | Result |
| --- | --- | --- | --- |
| Login and layout | Log in through GDM. Compare the layout with joe’s GNOME session at the same resolution. Open an app from Start, a file from a desktop icon and a menu from a tray icon. | The taskbar is along the bottom. Start, tray and desktop icons are easy to find and each action opens the expected item. | |
| Window controls | Click a GTK window’s own minimize and maximize buttons. Restore it from the taskbar, maximize and restore again. Repeat with a GPUI app and a server-decorated window. | Each window has one set of controls. Minimizing hides it, the taskbar restores it, maximizing leaves the taskbar usable and restoring recovers its previous bounds. | |
| Placement and motion | Drag slowly near an edge and a neighbouring window, then release far from either. Toggle reduced motion during a snap and repeat. | The window tracks the pointer. A nearby release settles quietly into alignment, a distant one stays put. Reduced motion reaches the same target immediately without bounce or fade. | |
| Files and menus | In Files and Nautilus, browse a folder, open a file from removable media, edit and save it. Open a menu near a screen edge, use a submenu, dismiss with Escape, then rename a file. | Menus stay reachable, Escape returns focus, the correct handler opens and the saved content and new name survive reopening. | |
| Terminals and clipboard | Open the suite’s Terminal. Copy non-ASCII text both ways between a native terminal and xterm. Try middle-click paste and drag a text selection between two accepting apps. | Text arrives intact once in the intended window. Middle click pastes the primary selection. Closing the source leaves the destination responsive. | |
| Input methods | Use an installed IBus engine in Text Editor and a Firefox form. Compose at two caret positions, choose a candidate, then switch apps mid-preedit. | Preedit is visible, candidates follow the caret and the chosen text commits once. The next field starts clean. | |
| Firefox and portals | Play audio. Request a screen share with two distinct displays, cancel, then select one. Stop from the shell. Request a screenshot and try a file chooser. | Audio uses the chosen output. Cancellation delivers no stream. The peer sees only the chosen display, the indicator stays visible while sharing and frames stop on request. The screenshot and chosen file match the selection. | |
| Displays | Drag GTK, Firefox and Files between displays at 100%, 150% and 200%. Click a small target and open a menu at the outer edge. Unplug the focused display and reconnect it. | Text and hit targets remain aligned, menus stay reachable and the window, pointer and keyboard remain usable on the surviving display. | |
| Display recovery | Try an advertised mode and rotation. Let the 15-second trial expire. Then try a layout with a display to the left of the primary, confirm it and reconnect that display. | The first trial restores the old resolution, rotation and pointer access. The confirmed layout survives reconnect and login. Moving left across the boundary reaches the expected window. | |
| Shell and settings | Switch workspaces, activate a notification, change volume and audio output, toggle Wi-Fi and connect a Bluetooth device. Restart the panel. | Focus follows the chosen window, the OSD leaves typing focus alone, device state matches the services and panel restart preserves windows and workspaces. | |
| Appearance and accessibility | Change contrast, text size, appearance and reduced motion with GTK and GPUI apps open. Use only the keyboard and Orca to launch an app, change a setting and dismiss a dialog. | Both toolkits update live, all text fits and focus stays visible. Orca announces the focused control’s name, role, value and changes. Preferences persist after login. | |
| GDM session lifecycle | Select mundaneDE in GDM. Open Settings and cancel a protected timezone change, then repeat and authenticate. Type in a window before and after a VT switch and suspend. Log out twice, open an app each time, then select GNOME. | Cancel keeps the timezone and authentication applies it. Each login has working input and freshly drawn windows. Each logout returns to GDM. GNOME opens normally. | |
| Seat, lock and power | Switch VTs and back. Lock two displays, attach another and exercise locker loss/recovery. Suspend while locked, resume, authenticate and log out. Log in twice. | Every display stays covered through locker loss and resume. Only the locker receives input until authentication. Input, display and app launching work on each new login. | |
| Upgrade ready | With Packages closed, check for an available upgrade, then prepare it. Dismiss the notification, inspect the system menu and complete a successful restart. | A preview alone shows no restart prompt. Once prepared, the desktop says the upgrade is waiting for restart. The system menu retains the status and it clears after a successful boot. | |
| Upgrade recovery | On the recoverable image, run joe’s failed-upgrade boot fixture and log in to the restored system. Dismiss the alert, reopen the system menu and restart the shell. Separately exercise an explicit rollback. | The desktop identifies the failed target and the previous system now running. Details remain available without duplicate alerts. An explicit rollback produces no failed-upgrade alert. | |
| Session fallback | Select GNOME at the greeter, open the saved file, then log back in to mundaneDE. | Both sessions start, the same file is accessible and the new session has working input and app launch. | |

Record fixes and repeat affected checks on the candidate to be signed off.
A failed or blocked row keeps the gate open. The distribution may change the
default only after the maintainer records their decision for that candidate.

| Candidate image and revision | Automated results | Completed checklist and retests | Maintainer sign-off and date |
| --- | --- | --- | --- |
| | | | |
