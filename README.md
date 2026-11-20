

SmartTaskbar  <img src="https://github.com/ChanpleCai/SmartTaskbar/blob/main/logo/logo.png" width="24">
=====

**Windows 11 Rust build** · [Setup and feature documentation](Rust/README.md) · [GitHub releases](https://github.com/PlasticBuild/SmartTaskbar/releases)

The Rust build offers intelligent auto-hide, configurable reveal on each display, and a native full-width or floating taskbar. The original SmartTaskbar documentation is below.

[![Version](https://badge.fury.io/gh/ChanpleCai%2FSmartTaskbar.svg)](https://github.com/ChanpleCai/SmartTaskbar/releases/download/v1.4.5/SmartTaskbar_Setup.exe)
[![Latest Release](https://img.shields.io/github/downloads/ChanpleCai/SmartTaskbar/latest/total.svg)](https://github.com/ChanpleCai/SmartTaskbar/releases/download/v1.4.5/SmartTaskbar_Setup.exe)
[![All Releases](https://img.shields.io/github/downloads/ChanpleCai/SmartTaskbar/total.svg)](https://github.com/ChanpleCai/SmartTaskbar/releases)
[![License](http://img.shields.io/:license-MIT-blue.svg?style=flat)](LICENSE)
[![996.icu](https://img.shields.io/badge/link-996.icu-red.svg)](https://996.icu)
[![LICENSE](https://img.shields.io/badge/license-Anti%20996-blue.svg)](https://github.com/996icu/996.ICU/blob/master/LICENSE)

* SmartTaskbar is a lightweight utility which can automatically switch the display state of the Windows Taskbar.

## Windows desktop preview

**Intelligent auto-hide** keeps the taskbars visible on the desktop and animates the affected bar when a window overlaps it, is maximized on that display, or goes fullscreen. A visible maximized or fullscreen window keeps its bar hidden when you switch to the other display. Move to its edge to reveal it temporarily. Maximized windows can use the taskbar strip without moving the desktop icons. SmartTaskbar restores the normal Windows work area on exit or pause.

**Always auto-hide** keeps the taskbars hidden until you pause the pointer near the edge. On stacked displays, pause just above the shared edge to reveal the upper taskbar. A quick crossing leaves SmartTaskbar's reveal inactive.

Intelligent auto-hide is the default for new settings. Use the tray menu to switch modes, or double-click the tray icon to pause or resume intelligent auto-hide. For access on secondary displays, enable **Show my taskbar on all displays** in Windows Settings. Explorer controls which taskbar positions support native auto-hide.

Press **Ctrl+Alt+Shift+T** or choose **Show both taskbars** in the tray menu to pause SmartTaskbar and restore the visible Windows taskbars. Turning off Windows auto-hide in Settings also pauses SmartTaskbar if it was using auto-hide. Use the tray menu to turn **Intelligent auto-hide** on again. The `SmartTaskbar.exe --show-taskbars` command is available for a desktop or Start menu shortcut.

In tablet posture, Explorer controls the taskbar and its touch gestures. SmartTaskbar resumes intelligent auto-hide when Windows reports laptop posture. The recovery process restores the original taskbar regions if SmartTaskbar closes unexpectedly. SmartTaskbar also rediscovers the taskbars when Explorer restarts.

The x64 portable build requires the [.NET 10 Desktop Runtime](https://dotnet.microsoft.com/download/dotnet/10.0). Extract the ZIP and run `SmartTaskbar.exe`. Use **Exit** in the tray menu to close it.

```powershell
dotnet publish Sources/SmartTaskbar/SmartTaskbar.csproj -c Release -r win-x64 --self-contained false
dotnet run --project Sources/SmartTaskbar.Tests/SmartTaskbar.Tests.csproj -c Release
```

### Taskbar appearance

Open **SmartTaskbar Appearance** to choose **Full width** or **Floating island**. Set the corner radius from 0 to 24 px and the glass transparency from 0 to 85%, then select **Apply**. The island groups Windows Start, app buttons and the system tray. Search follows your setting in **Windows Settings → Personalization → Taskbar**. **Restore Windows look** returns to the standard taskbar style.

The live preview uses your Windows taskbar app icons and follows the Windows theme. Choose **Simulate fullscreen** to see the taskbar slide away, then move the pointer to the preview's bottom edge to try the selected activation zone. Hover and click app icons in the preview to try their motion. Select **Apply changes** to update the real taskbar.

**Configure Start web searches** opens the optional browser redirection settings. Redirection is off by default. Enabling it downloads a checksum-verified portable copy of MSEdgeRedirect and starts it for the current user. Choose a provider for web and image search links opened in your default browser. Turning it off stops the managed helper and removes its sign-in entry. Windows supplies the thumbnails shown inside Start through its own search provider. [MSEdgeRedirect](https://github.com/rcmaehl/MSEdgeRedirect) · [Windows Search providers](https://learn.microsoft.com/en-us/windows/apps/develop/search/search-providers)

**Hover lift** raises and enlarges app icons while the pointer is over them. **Click depth** lowers and shrinks the icon while you hold the mouse button. Set either value to 0 to turn off that SmartTaskbar effect.

**Activation zone** controls where Intelligent auto-hide responds along each display edge. Choose the entire edge, the center half, or the taskbar's visible width. In Island layout, clicks beside the bar reach the window underneath. A taskbar flyout keeps its originating bar visible while it is open.

With a bottom island, choose **Left**, **Center**, or **Right** in SmartTaskbar Appearance. Start follows the island's width as app buttons appear or disappear. Its width stays between 420 and 760 device-independent pixels where the screen permits. Quick Settings and Notification Center open at the tray end. The calendar expands within the notification panel. **Animate menus** controls the transition between these flyouts and the island.

Open **SmartTaskbar Appearance** from the Desktop or Start menu. Under **Intelligent auto-hide**, **Start SmartTaskbar when I sign in** controls whether SmartTaskbar starts and restores your selected appearance at Windows sign-in.

When Windows places the taskbar on the left or right edge, the island wraps the vertical stack of controls on each display. Clicks above and below it reach the window underneath. The primary display includes the system tray.

The picker runs independently of Intelligent auto-hide. Styling uses a portable Windhawk instance scoped to Explorer and the native menu hosts. [Windhawk release](https://github.com/ramensoftware/windhawk/releases/tag/2.0.0-alpha.8) · [Taskbar Styler](https://windhawk.net/mods/windows-11-taskbar-styler) · [Start Menu Styler](https://windhawk.net/mods/windows-11-start-menu-styler) · [Notification Center Styler](https://windhawk.net/mods/windows-11-notification-center-styler) · [Shell Flyout Positions](https://windhawk.net/mods/shell-flyout-positions)

Publish the Appearance project for Windows x64 into the Appearance directory, then run **Sources/SmartTaskbar.Appearance/Prepare-Windhawk.ps1** with `-OutputDirectory ./Appearance -CreateShortcuts` to prepare the styling components and create both shortcuts.

## Earlier releases

#### Auto Mode

* In the Auto Mode, SmartTaskbar will set the Taskbar to hide when When the focused window and the taskbar intersect<sup>[[1]](#footnote)</sup>.
  
* Double-click the tray icon to switch the display status of the taskbar between Show or Auto-Hide.

Known Issues
----  

* Some applications are not compatible with Auto-Hide mode (This problem has nothing to do with SmartTaskbar), for example:
  
    * Some applications use special maximization logic. When the mouse is moved to the taskbar position, the taskbar doesn't pop up. In this case, you can use the shortcut (`WIN` + `T`) to force the taskbar to pop up.

* Starting from [v1.4.3](https://github.com/ChanpleCai/SmartTaskbar/releases), the following behavior only occurs when the hook fails.
>* The Auto Mode is based on Auto-Hide mode, so it does not change the default behavior of window taskbar in Auto-Hide mode. Therefore, you will encounter the following "bugs", but they are not actually:
>
>    * When you close the start menu or the search panel, the taskbar will be automatically hidden (this is the system's own behavior), and then may be displayed again immediately (this is the work of SmartTaskbar).
>   
>    * When there is already a full-screen application, open a window that does not intersect the taskbar, and the taskbar will be automatically hidden when the mouse is moved away (this is the system's own behavior), and then it may be displayed again immediately (this is SmartTaskbar working ), since the currently focused window does not intersect the taskbar.

Build
-----
* Visual Studio 2022.

Notice
------
* <a name="footnote"> The status of the taskbar does not change when the mouse is over the taskbar.</a>  

* The [Microsoft Store](https://www.microsoft.com/en-us/p/smarttaskbar/9pjm69mps6t9?activetab=pivot%3aoverviewtab) version is slightly less functional and stable.
