# Road to a joe Linux desktop session

mundaneDE aims to replace GNOME 51 as the default desktop on x86 joe Linux.
Today it can run a Wayland client in a host window or on one DRM/KMS output.
It has spring-driven window placement, a magnetic snap preview, wallpaper
crossfades and a separate settings application. GDM, GNOME Shell and Mutter
still provide the distribution's desktop session.

These milestones are planned. Each ends with something a user or tester can
observe. GTK4/libadwaita applications and Firefox are compatibility targets
throughout the transition.

| Milestone | Acceptance test |
| --- | --- |
| 1. Repeatable nested session | A real Wayland client opens, resizes and closes in an isolated run. The first frame shows Afterlight, the final frame clears the client, and a cold start on the affected EGL driver reports no buffer-age error. |
| 2. Everyday applications | Use menus and dialogs in Files, compose text with IBus in GTK and Firefox, and copy text between a native Wayland window and an Xwayland application. |
| 3. Outputs and animation | Drag a window between 100% and 150% displays, unplug one and keep the window reachable. Meet the frame and idle budgets on the recorded reference hardware. |
| 4. Login and logout | Select mundaneDE in GDM, switch VTs, suspend, resume and log out twice. Each login gets working seat devices and a fresh shell environment. GNOME remains selectable. |
| 5. Desktop shell and lock | Launch an app, switch workspaces, respond to a notification and change a device from the system menu. Lock both displays and disconnect the locker. Every display stays covered until authenticated recovery. |
| 6. Desktop portals | In Firefox, choose a display, share it over PipeWire and stop sharing. Cancelled requests return no stream. A screenshot of a known scene matches that scene. |
| 7. Settings and accessibility | Change appearance in Settings and see it update in GTK and GPUI. Use the launcher and a settings dialog by keyboard and with Orca. Reduced motion takes effect on the next frame. |
| 8. Default-session qualification | On a clean x86 image, complete first login, daily app workflows, lock/resume and logout. Repeat after upgrade and demonstrate recovery by selecting GNOME in GDM. |

The [implementation plan](docs/plan.md) gives the protocol dependencies, test
fixtures and handoff for each milestone. The GPUI suite delivers Terminal and
Files independently. Existing GNOME applications remain available until their
replacements pass the corresponding application tests. [Application suite plan](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/main/docs/plan.md)
