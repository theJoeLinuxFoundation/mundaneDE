# Authentication before and after

Each image is a full 1920×1080 QMP scanout from a headless joe beta1 guest. In both versions the authority offered two administrators and the operator selected joe. The earlier captures show the action details. The new captures show the default collapsed state and the desktop dimmed behind the prompt.

| Appearance | Step 4 | Redesigned prompt |
| --- | --- | --- |
| Light | [![Previous light dialog](../gpui-step4/light.png)](../gpui-step4/light.png) | [![Compact light dialog with Authenticate selected](light.png)](light.png) |
| Dark | [![Previous dark dialog](../gpui-step4/dark.png)](../gpui-step4/dark.png) | [![Dark modal with joe’s initials and login](dark.png)](dark.png) |
| High contrast | [![Previous high-contrast dialog](../gpui-step4/contrast.png)](../gpui-step4/contrast.png) | [![High-contrast modal with a focused password field](contrast.png)](contrast.png) |

## GPUI lock

| Light | Dark | High contrast |
| --- | --- | --- |
| [![Light lock screen](locked-light.png)](locked-light.png) | [![Dark lock screen](locked.png)](locked.png) | [![High-contrast lock screen](locked-contrast.png)](locked-contrast.png) |

The guest exercised an incorrect password with Orca listening. [Inline PAM denial](denied.png)
