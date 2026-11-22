# Maintainer’s desktop hand test

Use the **QEMU** rows for the early hand test. Repeat them on the packaged
candidate after automated qualification passes on clean and upgraded joe images.
**Hardware** identifies checks reserved for the packaged machine. If a QEMU
row needs an app or input engine, use it in the guest when installed or repeat
it on the candidate. The standard trial window has one active virtual display,
a keyboard and a pointer. It has no
network adapter, audio device, physical radios, battery or backlight. A missing
device or unavailable test app should be recorded as **blocked**, with the reason.

For an earlier desktop trial, run `./tests/try-mundane.sh` in a clean clone.
It fetches the published references from `origin` and builds an export of
`origin/main` in the cache. The terminal and the QEMU window title show the
full commit ID. Use `--revision COMMIT` to repeat a build. Both the checkout
commit and the selected revision must be reachable from a published origin
branch or tag. A dirty checkout or an unpublished commit requires
`--from-working-tree`, which labels the build as a working tree.

The visible guest keeps its disk in `~/.cache/mundane-try/desktop`. If you set
`MUNDANE_TRY_DIR`, use that same directory to resume your trial. Headless
qualification defaults to `~/.cache/mundane-try/qualification`. The guest
opens at 1920×1080. Log in or unlock as `joe` with password `joe`. Quit with
**Quick Settings → Shut Down**.
[Guest setup and controls](../tests/qemu/session/README.md#try-the-desktop)

Record the date, tester, full revision from the window title, image ID, suite
revision, GPU/driver, displays, scales and input devices. For each row record
**pass**, **fail** or **blocked**, what happened and any issue or retest. Keep
the QEMU and packaged-machine results separately. Attach the automated results
to the final decision.

## GPUI shell

| Where | Area | What to try and what good looks like | Result |
| --- | --- | --- | --- |
| QEMU | Login and taskbar | Log in through GDM at 1920×1080. Compare with joe’s GNOME layout. Start is at bottom left, pinned and running apps along the bottom, tray and clock at the right. Maximize a window. Its controls and the whole taskbar stay reachable. | |
| QEMU | Start and pins | Click Start and try Super+Space. Search by app name and keyword, browse Utilities, then launch with Enter. Find an app in both the pinned grid and the full list. Right-click, press Menu on a focused app and try a long press. Each opens one context menu. Escape restores focus without launching. Pin and unpin. The list’s star and the taskbar agree and pin order survives logout. | |
| QEMU | Running apps and switching | Launch a pinned app, click its pin to focus it, then open two documents in that app. The running dot and active state distinguish the pin. The group offers both titles. Minimize one, restore it from the taskbar and close the other. Hold Alt and cycle with Tab and Shift+Tab. Escape preserves the original focus. Releasing Alt focuses the highlighted window. | |
| QEMU | Workspaces | Open the workspace selector. Put a window on Desk 2 and leave Desk 3 empty. Check hover names, active and occupied markers. Switch by click, wheel, keyboard and Super+1–4. Use the window finder to move one window and activate it. The other windows stay put. Repeat with reduced motion. The target appears immediately. | |
| QEMU | Tray and menu dismissal | With a StatusNotifier item running, activate its main action and a submenu action using pointer and keyboard. Each reaches the owner once. In an editor, open Start, Quick Settings, Workspaces and the tray menu in turn. Dismiss by second activation, Escape and clicking outside on an editor control. The control stays untouched and typing resumes at the previous caret. | |
| QEMU | Quick Settings | Click the clock, then try Super+S. Navigate the device tiles, volume, history and power row by keyboard. Hover or focus Lock, Log Out, Restart and Shut Down and check their names. Toggle Do Not Disturb. Cancel Restart and Shut Down using Enter on the initially focused Cancel button. Repeat at a smaller logical size and in high contrast. The panel stays on-screen and all controls remain reachable. | |
| QEMU | Notifications and history | In a guest terminal run `notify-send "Hand test" "First message"`. Find it in Quick Settings history, then enable Do Not Disturb and send a second message. The second goes into history quietly. Use the arrows to select and dismiss one card. Try an app notification with an action. The action reaches the app once, the other card remains and a banner leaves typing focus alone. | |
| QEMU | Shell authentication | Open a protected timezone change in Settings. The desktop dims and ignores clicks. Details starts closed and expands to show the action. The prompt shows the administrator’s initials or avatar, name and login. Change identity if several are eligible. Authenticate is the primary button. Enter a wrong password, then retry with Enter. The inline denial shakes only with motion enabled. Escape or locking mid-prompt cancels the change. A fresh authorized change succeeds and returns focus to Settings. | |
| QEMU | Lock screen | Set a wallpaper, then use Super+L with an editor open. Repeat in light, dark and high contrast. See the dimmed wallpaper, clock/date, identity, keyboard layout and centred password field. No app pixels or notification text appear. After 30 seconds only the clock remains. Wake with a key or the pointer. The wake key leaves the entry empty. A bad password stays locked. Enter with the correct password restores the editor. Reduced motion removes the fade. | |
| QEMU | Desktop icons | Create, rename and delete a file in the Desktop folder. Give it a long name. The centred idle label uses two lines with an ellipsis only on the second. Hover and selection reveal the full name. Use Super+D, navigate with arrows, open with Enter and try Home and Trash. The preferred app opens. Super+D again restores the workspace. [Idle](screenshots/gpui-step7/icon-label.png) · [Hover](screenshots/gpui-step7/icon-label-hover.png) | |
| QEMU | Accessibility controls | Use Ctrl+Alt+Tab to focus the taskbar, reach Start and launch an app by keyboard. Use Super+Alt+S to toggle the reader, Super+, for Settings and Space to toggle High contrast. Focus is visible and settings persist at next login. Check audible speech on the hardware rows below. | |
| QEMU with a guest volume key | Volume OSD | If Quick Settings has no output, create a guest sink with `pactl load-module module-null-sink sink_name=handtest` and select it. In an editor, send a volume or mute key to the guest and keep typing. The level changes and the overlay disappears without stealing the caret. Remove the sink with `pactl unload-module ID` using the number printed at creation. If the host intercepts the key, record that part for hardware. | |
| Hardware | OSDs and device controls | While typing, press volume, mute and brightness keys. The level OSD shows each new value, disappears and leaves every keystroke in the editor. Switch audio sinks, unplug the selected sink and try the sliders. Toggle Wi-Fi, cancel and confirm Bluetooth pairing, then reconnect. Optional controls and battery state match the devices. | |
| Hardware | Orca speech and credentials | With working audio, enable Orca and character echo. Listen as you use Start search, a Settings toggle, notification history and an app notification. Check names, roles, states and focus. In a GTK and a GPUI test entry, type `z` after `Tokyo`, then in a fresh trial have the app append `z` to the focused entry. Both show `Tokyoz`. Only typing speaks “z”. In the authentication and lock prompts Orca reads the field and denial without speaking the password. Log out and verify speech at the next login. | |
| Hardware | Bars, lock and output changes | Connect two displays. Check a taskbar, desktop icons and finder on each. Move the pointer across them while locked. Both show the wallpaper and clock, with the prompt on the pointer’s screen. On a recoverable test installation exercise locker loss, attach a third display, switch VTs and suspend/resume while locked. Every active head stays covered. Input returns to the app only after successful authentication. | |
| QEMU | Shell recovery | Open two documents on different desks and restart the shell in the guest (`pkill -TERM -x mundane-shell` as joe). The supervised panel returns. Workspaces and tasks retain their windows and the new panel can activate and close them. | |

[Lock and desktop icons](screenshots/gpui-step6/README.md) · [Authentication](screenshots/gpui-step5/README.md) · [Start and Quick Settings](screenshots/gpui-step2/README.md) · [Notifications and sharing](screenshots/gpui-step3/README.md)

## Applications, session and upgrades

| Where | Area | What to try and what good looks like | Result |
| --- | --- | --- | --- |
| QEMU | Window controls and placement | Use a GTK window’s own minimize and maximize buttons, restore it from the taskbar and repeat with Settings and a server-decorated app. There is one set of controls and restoring recovers the old bounds. Drag near an edge or neighbour and release, then release far away. Only nearby releases snap. Toggle reduced motion during a snap. The same position is reached without bounce. | |
| QEMU | Files, menus and clipboard | In Nautilus browse a local folder, edit, save, rename and reopen a file. Open a submenu near a screen edge and dismiss it with Escape. Focus returns and menus stay reachable. Copy non-ASCII text both ways between Wayland and X11 apps, middle-click the primary selection and drag between accepting apps. Text arrives once and survives in the destination after the source closes. | |
| Hardware | Packaged suite and removable media | Repeat the file and clipboard checks with the suite’s Files and Terminal, xterm and a removable drive on the candidate. Verify the intended handler opens the saved file after reconnecting the drive. | |
| QEMU | Live appearance | Put Settings beside a libadwaita app. Change light/dark, accent, contrast, text size and reduced motion, including during a wallpaper transition. Open controls and taskbar update live, labels fit and keyboard focus stays visible. Open and cancel Password and Add remote in Settings. The masked field stays private and Escape leaves the setting alone. | |
| Hardware | Appearance across toolkits and displays | With the packaged suite Terminal, Files, Firefox and a GTK app open, repeat the appearance check at 100%, 150% and 200%. Move them between monitors and click small targets near an edge. Controls, Terminal grid/cursor and hit targets remain aligned. | |
| QEMU | Portals | Request a screenshot and a local Firefox screen share from a portal-capable client. Cancel has initial focus. Cancel, then select the display or a single window and cover that window. Check the PNG’s colours and orientation. The image and stream show only the selected content, the taskbar names the share and Stop ends it. Repeat and close the source, quit Firefox or lock. The indicator clears and unlocking requires fresh consent. | |
| Hardware | Share with a peer | Make a Firefox WebRTC call with audio. Choose between two distinct displays. Stop from the other taskbar, repeat and unplug the shared display. The peer receives the selected display with consent and never sees the lock screen. Each stop clears both indicators. | |
| QEMU with a sandboxed caller | Document grants | In a sandboxed app, change a chooser filter, cancel, then select and save a file. Cancellation grants nothing, replace requires confirmation and the chosen URI handler opens. | |
| QEMU with a conversion engine | Input methods | Use the installed IBus engine in a GTK entry and a local Firefox form. Preedit is inline, candidates follow the caret, a click commits once and switching apps clears preedit. | |
| Hardware | Display recovery | Try rotation and an advertised mode. Let the 15-second trial expire. Then confirm a layout with a display left of primary, unplug the focused display and reconnect. The timed trial restores the previous layout. Confirmed positions survive reconnect and login, and the surviving display keeps pointer and keyboard access. | |
| QEMU | Logout and GDM fallback | Save a file and log out twice. Select mundaneDE again, then GNOME at the greeter. Each fresh session draws windows, launches apps and accepts input. The saved file opens in both. | |
| Hardware | Power and seat | Before and after a VT round trip, test pointer and typing. Suspend with the lid, resume and authenticate. Repeat with video inhibiting idle and lock explicitly with Super+L. The lock remains effective, and fresh GDM logins regain seat devices. | |
| Hardware | Upgrade ready | On a recoverable candidate with current dave, check and then prepare an upgrade with Packages closed. Only preparation shows “Upgrade ready, restart to apply”. Dismiss, restart the shell and reprepare the same set. There is one banner per prepared ID, including a distinct upgrade with the same counts. Quick Settings retains status, restart offers Cancel, and the notice clears after successful boot. | |
| Hardware | Upgrade recovery | Use joe’s failed-upgrade boot fixture, log in to the restored system and reopen Quick Settings. The alert names the attempted and restored systems once per event. Details persist after dismissal and shell restart. A separate explicit rollback gives no failure alert. | |

## Retest the first hand-test findings

The maintainer found these three issues on `5332dbc`. Repeat the checks in
the same app you used for the first trial. Record your observed result and
candidate commit in the last column. The automated retest passed on a headless
joe beta1 guest for all three rows. [Before and after at 1920×1080](screenshots/shell/README.md)

| Where | Finding on `5332dbc` | What to try on the candidate | Automated coverage | Maintainer retest |
| --- | --- | --- | --- | --- |
| QEMU | An open menu stayed open on a second click. | Focus an editor. For Start, the clock, Workspaces and a tray item with a menu, open it and click its button again. Repeat with Escape and then with a click on an editor control. Type after each dismissal. | The guest checks that each menu closes, the outside click leaves the control untouched and typing reaches the editor again. | Pending |
| QEMU for layout and confirmations, hardware for device readings | The system menu made device controls and power actions hard to pick out. | Open Quick Settings with the clock and with Super+S. Read and toggle Wi-Fi, Bluetooth and Do Not Disturb. Adjust volume and laptop brightness with the keyboard. Compare battery state with the machine. Hover or focus each power icon, then open and cancel Restart and Shut Down. Repeat in high contrast and on a smaller display. | The guest exercises radio state, notification history, the volume slider and both confirmations. All power buttons fit at 960×540 logical pixels in high contrast. A hardware fixture checks optional battery, backlight and Bluetooth controls. | Pending |
| QEMU | The workspace selector needed clearer state, feedback and navigation. | Put a window on Desk 2 and leave Desk 3 empty. Hover the workspace markers, open the selector, switch with the mouse, wheel and keyboard, then repeat with reduced motion. | The guest checks current and occupied states, input paths and intermediate scanout frames. Reduced motion switches directly to the destination. | Pending |

Record fixes and repeat affected checks on the candidate to be signed off.
A failed or blocked row keeps the gate open. The distribution may change the
default only after the maintainer signs off on the packaged candidate on a real machine.

| Candidate image and revision | Automated results | Completed checklist and retests | Maintainer sign-off and date |
| --- | --- | --- | --- |
| | | | |
