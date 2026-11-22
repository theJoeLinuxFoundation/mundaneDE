# Road to a joe Linux desktop session

mundaneDE aims to replace GNOME 51 as the default desktop on x86 joe Linux.
Today it can run a Wayland client in a host window or across DRM/KMS outputs with different scales and refresh rates.
It has spring-driven window placement, a magnetic snap preview, wallpaper
crossfades and a separate settings application. GDM, GNOME Shell and Mutter
still provide the distribution's desktop session.

Milestones 1–3 pass in isolated nested sessions and a two-output QEMU DRM guest. The remaining milestones each end with a user-visible
acceptance test. GTK4/libadwaita applications and Firefox are compatibility targets
throughout the transition. The desktop will keep joe’s familiar layout and report
dave upgrade and recovery status itself. [Public announcement](https://joelinux.io/news/mundanede/)

| Milestone | Acceptance test |
| --- | --- |
| [1. Repeatable nested session](tests/nested/README.md) | A real Wayland client opens, resizes and closes in an isolated run. The first frame shows Afterlight, the final frame clears the client, and a cold start on the affected EGL driver reports no buffer-age error. |
| [2. Everyday applications](tests/nested/README.md#everyday-applications) | Use GTK’s own buttons to minimize, maximize and restore a window. Rename a file in Nautilus, compose text with IBus in Text Editor and Firefox, and paste in both directions between native and X11 terminals. |
| [3. Outputs and animation](tests/qemu/README.md#automated-output-test) | Drag a window between 100% and 150% displays, unplug one and keep the window reachable. Check that magnetic snapping feels unobtrusive and still works with reduced motion. Meet the frame and idle budgets on the recorded reference hardware. |
| 4. Login and logout | Select mundaneDE in GDM, switch VTs, suspend, resume and log out twice. Each login gets working seat devices and a fresh shell environment. GNOME remains selectable. |
| 5. Desktop shell and lock | Use the bottom taskbar, Start menu, tray and desktop icons. Minimize, restore and maximize a window using its buttons. Verify the shell reports a prepared dave upgrade awaiting restart and an automatic return to the previous system after a failed upgrade. Lock both displays and disconnect the locker. Every display stays covered until authenticated recovery. |
| 6. Desktop portals | In Firefox, choose a display, share it over PipeWire and stop sharing. Cancelled requests return no stream. A screenshot of a known scene matches that scene. |
| 7. Settings and accessibility | Change appearance in Settings and see it update in GTK and GPUI. Use the launcher and a settings dialog by keyboard and with Orca. Reduced motion takes effect on the next frame. |
| 8. Default-session qualification | Qualify clean and upgraded x86 images, including app workflows, lock/resume, upgrade recovery, the GNOME fallback and joe’s tests under security logging and enforcement. After automation passes, the maintainer tests the candidate on a real machine using the [manual checklist](docs/manual-test.md). Changing the default requires their sign-off. |

The [implementation plan](docs/plan.md) gives the protocol dependencies, test
fixtures and handoff for each milestone. The GPUI suite delivers Terminal and
Files independently. Existing GNOME applications remain available until their
replacements pass the corresponding application tests. [Application suite plan](https://github.com/theJoeLinuxFoundation/mundane-apps/blob/main/docs/plan.md)
