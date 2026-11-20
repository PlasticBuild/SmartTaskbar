

SmartTaskbar  <img src="https://github.com/ChanpleCai/SmartTaskbar/blob/main/logo/logo.png" width="24">
=====
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

The picker runs independently of Intelligent auto-hide. Styling uses a portable Windhawk instance and Windows 11 Taskbar Styler scoped to Windows Explorer. The selected layout resumes at sign-in. [Windhawk release](https://github.com/ramensoftware/windhawk/releases/tag/2.0.0-alpha.8) · [Taskbar Styler](https://windhawk.net/mods/windows-11-taskbar-styler)

Publish the Appearance project for Windows x64 into the Appearance directory, then run **Sources/SmartTaskbar.Appearance/Prepare-Windhawk.ps1** with **-OutputDirectory ./Appearance**. Open **Appearance/SmartTaskbar.Appearance.exe**.

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
