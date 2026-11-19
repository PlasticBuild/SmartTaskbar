

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

**Intelligent auto-hide** keeps the taskbars visible on the desktop and while windows leave their space clear. Fullscreen can cover a taskbar on its own display while the other taskbar stays accessible. An ordinary window that overlaps the taskbar triggers auto-hide, and the taskbars return when the space clears.

**Always auto-hide** keeps the taskbars hidden until you pause the pointer near the edge. On stacked displays, pause just above the shared edge to reveal the upper taskbar. A quick crossing leaves SmartTaskbar's reveal inactive.

Intelligent auto-hide is the default for new settings. Use the tray menu to switch modes, or double-click the tray icon to pause or resume intelligent auto-hide. For access on secondary displays, enable **Show my taskbar on all displays** in Windows Settings. Explorer controls which taskbar positions support native auto-hide.

Press **Ctrl+Alt+Shift+T** or choose **Show both taskbars** in the tray menu to pause SmartTaskbar and restore the visible Windows taskbars. Turning off Windows auto-hide in Settings also pauses SmartTaskbar if it was using auto-hide. Use the tray menu to turn **Intelligent auto-hide** on again. The `SmartTaskbar.exe --show-taskbars` command is available for a desktop or Start menu shortcut.

SmartTaskbar rediscovers the taskbars after Explorer restarts. When **Show taskbar on exit** is enabled, the app gives Explorer a short time to finish hiding an open bar before it restores the visible taskbars.

The x64 portable build requires the [.NET 10 Desktop Runtime](https://dotnet.microsoft.com/download/dotnet/10.0). Extract the ZIP and run `SmartTaskbar.exe`. Use **Exit** in the tray menu to close it.

```powershell
dotnet publish Sources/SmartTaskbar/SmartTaskbar.csproj -c Release -r win-x64 --self-contained false
dotnet run --project Sources/SmartTaskbar.Tests/SmartTaskbar.Tests.csproj -c Release
```

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
