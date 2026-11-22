# A day on joe’s desktop

The reference is joe’s GNOME 51 x86 desktop with Dash to Panel 74, ArcMenu
70, AppIndicator and Desktop Icons NG 51.4. The taskbar runs along the bottom.
joe pins Firefox, Files, Terminal, Text Editor and Calculator in that order.
The clock and system menu are separate buttons. The table starts from
mundaneDE `2a0c8d0`, before the changes below. [joe’s desktop defaults](https://github.com/theJoeLinuxFoundation/joe-linux/blob/5dc2ac8/packages/joe-desktop/build.sh) · [Desktop packages](https://github.com/theJoeLinuxFoundation/joe-linux/blob/5dc2ac8/packages/joe-desktop/package.zon)

The rank estimates how often someone who browses, edits files and moves among
several windows would encounter the difference. It is a workflow estimate,
without usage telemetry. A laptop user who changes brightness on the move or
relies on closing the lid may put those rows first.

| Rank and likely frequency | On joe’s GNOME desktop | On mundaneDE at the baseline |
| --- | --- | --- |
| 1 · many times an hour | Hold Alt and tap Tab to switch, Shift to go backwards. The switcher shows the choice before releasing the modifier. GNOME can group windows by app. | Switching requires a taskbar click or the window finder. Super+Tab restores a minimized window. Alt+Tab gives no shell feedback. |
| 2 · many times a day | A favourite has a stable taskbar position before its first window opens and after its last window closes. Use it to launch or return to the app. | The initially empty taskbar gains a titled button for each open window on the current workspace. A favourite needs another trip through Start. |
| 3 · several times a day | ArcMenu offers pinned apps, search and category browsing. It helps when the user remembers an app’s purpose better than its name. | Start has a searchable alphabetical list. It matches the display name and desktop ID. It has no favourites view, keyword search or categories. |
| 4 · often with multiple documents | Dash to Panel groups an app’s windows and offers hover previews so two similarly titled documents can be distinguished. | Each window has a text button. Workspaces offers title search and explicit activate/move/close. There are no taskbar thumbnail previews. |
| 5 · several times a day | The clock opens the notification list. Missed banners, grouped app notifications, clearing the list and Do Not Disturb are easy to reach. | Banners and actions work. Expand Notifications in Quick Settings to revisit and dismiss up to 64 entries, including ones received in Do Not Disturb. History lasts for the panel process. There is no clock-side notification centre. |
| 6 · several times a day on a laptop | Volume, mute and brightness keys show immediate graphical feedback without taking focus. | Volume up/down adjusts the sink and shows its raw wpctl reading in a toast. Quick Settings has volume and available backlight sliders. Mute and brightness keys lack equivalent feedback. |
| 7 · daily for keyboard users | Familiar shortcuts reach the overview/launcher, window switching, lock, notifications and capture. GNOME Settings lets users discover and rebind shortcuts. | Super+Space opens Start, Super+S opens Quick Settings, Super+D shows the desktop, Super+1–4 selects desks, Super+L locks, Super+, opens Settings and Super+Return opens a terminal. There is no shortcut editor or general GNOME binding import. |
| 8 · daily for calendar users | Clicking the time shows a month grid and available appointments beside notifications. | The bar shows hours and minutes. Clicking it opens Quick Settings. There is no month grid, date navigation or agenda. |
| 9 · daily on mobile hardware | GNOME’s power service coordinates dimming, idle suspend and power settings with logind; lid behaviour depends on docking, inhibitors and system policy. | Lock, idle inhibition and the suspend/resume handshake work. Panel actions reach logind/systemd. Lid handling follows joe’s logind policy. Physical lid, dock and battery behaviour still need the hardware gate. There is no desktop lid or idle-suspend preferences page. |
| 10 · each evening if enabled | Night Light adjusts display colour temperature on a schedule. | Appearance and contrast change live. There is no sunset scheduler or per-output colour-temperature adjustment. |
| 11 · a few times a week | Print opens an interactive screenshot/recording tool with area or display selection; a finished recording is a video file. | Portals let an app request a screenshot or PipeWire stream of a window or display, with consent, indicator and Stop. There is no desktop Print chooser, rectangle selection or shell command to save a recording. |
| 12 · when connecting media | The shell can announce a removable volume and offer to open it, subject to removable-media preferences. Files exposes mounts and eject. | Installed file handlers and Files can open mounted volumes. The shell does not announce a newly mounted drive or offer safe eject. |
| 13 · once per account | joe’s first-account setup leads to GDM, the configured menu, five pins, tray and desktop icons. DING shows files on the desktop. | GDM can select mundaneDE for the same account. Home, Trash, desktop files, tray and Start appear immediately. joe owns initial account creation. There is no mundaneDE tour. Test the actual packaged first login in milestone 8. |

GNOME’s user guide describes [window switching](https://help.gnome.org/gnome-help/shell-windows-switching.html),
[shortcuts](https://help.gnome.org/gnome-help/shell-keyboard-shortcuts.html),
[notification history](https://help.gnome.org/gnome-help/shell-notifications.html)
and [screenshots and recording](https://help.gnome.org/gnome-help/screen-shot-record.html).
The [manual checklist](manual-test.md) tracks the packaged image and hardware sign-off.
