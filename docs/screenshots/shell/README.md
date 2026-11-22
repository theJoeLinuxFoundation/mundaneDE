# First hand-test retest

These full-resolution screens come from a headless joe Linux beta1 guest at
1920×1080. Workspace 1 is clear and Text Editor is open on workspace 2.
Select a thumbnail to inspect the original PNG.

| Hand-test finding | Before · `5332dbc` | After |
| --- | --- | --- |
| Start and menu dismissal | [![Original Start menu](before-start.png)](before-start.png) | [![Revised Start menu](after-start.png)](after-start.png) |
| Clock and system controls | [![Original system menu](before-quick-settings.png)](before-quick-settings.png) | [![Quick Settings with radio states, volume and power controls](after-quick-settings.png)](after-quick-settings.png) |
| Switching workspaces | [![Original workspace overview](before-workspaces.png)](before-workspaces.png) | [![Revised selector with current, occupied and empty workspaces](after-workspaces.png)](after-workspaces.png) |

Quick Settings also has a [high-contrast view with keyboard focus](after-high-contrast.png).

The screenshots show each menu open. The [headless test](../../../tests/qemu/session/qualify_menus.py)
drives the second click, Escape and outside click and checks where the next
keystroke goes. The [hand-test checklist](../../manual-test.md#retest-the-first-hand-test-findings)
tracks the maintainer’s retest.
