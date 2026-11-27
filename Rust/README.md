# SmartTaskbar for Windows 11 in Rust

The engine, settings window, recovery process, profile selection and optional integrations are implemented in Rust. Explorer supplies the native taskbar controls, and a private portable Windhawk host applies the appearance to Windows shell surfaces.

Run `SmartTaskbar-Setup.exe` on 64-bit Intel or AMD Windows 11. Setup installs for the current user, creates one **SmartTaskbar** entry in Start and Installed apps, and can add a desktop shortcut. Rerunning Setup upgrades that installation and keeps its preferences. Open **SmartTaskbar** in Start to change appearance, optional features and sign-in behavior.

Open `smarttaskbar-rs.exe` for settings. The preview lets you try the island, full-width bar, pointer effects, fullscreen reveal and menu transition. Select **Apply** to save. Preferences are stored beside the executable in `settings.json`, or in the file selected with `--config`. Turning on sign-in registers that executable for the current Windows user.

**Ctrl+Alt+Shift+T** restores the taskbar and exits the controller. `smarttaskbar-rs.exe --stop` also requests a graceful exit.

Use **Reveal settings for display** to edit defaults or choose a connected monitor and enable **Use custom reveal settings**. Each monitor can use its own activation zone and delays for outside edges and shared edges. The shared edge still requires an approach from inside that display.

**Pause SmartTaskbar** restores Windows control and stops the controller. SmartTaskbar stays paused across sign-in and upgrades until you select **Resume SmartTaskbar**. The **Updates** tab checks the public GitHub release feed when Settings opens. It shows availability inside Settings, with links to releases and the repository. A new installer is opened by the user.

**App icon size** scales pinned and running app icons from 70% to 125% using the same base size on horizontal and vertical taskbars. It updates the preview as you drag. Select **Apply** to use the new size.

Use **Match Start to island width** in Appearance to size the native Start menu to a bottom floating island within the panel size limits. This option is off by default. Windows supplies the Search interface and controls web results through its own settings.

Setup offers **MSEdgeRedirect (optional)**. The component is unselected by default. If you install it, use **Enable MSEdgeRedirect** under Appearance and select **Apply**. Use **Uninstall MSEdgeRedirect** in the same section to remove the component. Run Setup again to reinstall it.

Turn on **Show taskbar media player**, choose **Media player side**, and select **Apply**. The player appears when an app starts reporting playback to Windows. Available artwork fills the player background with a blur. The player offers play or pause, previous and next controls, and a scrubber. Seeking and track navigation depend on the app. The player follows the taskbar orientation and fits inline when there is room. On a vertical taskbar, Left chooses the upper end and Right chooses the lower end.

## Commands

| Command | Action |
| --- | --- |
| `--settings` | Open the settings window. This is the default. |
| `--run` | Run the controller for the current Windows session. |
| `--stop` | Restore the taskbar and stop the Rust controller. |
| `--pause` | Pause SmartTaskbar across sign-ins and restore the Windows taskbar. |
| `--resume` | Clear the pause and start SmartTaskbar. |
| `--inspect` | Read the current monitor and taskbar geometry. |
| `--check` | Validate the settings and generate shell profiles entirely in memory. |
| `--import-appearance PROFILE --config FILE` | Create a new settings file from a previous SmartTaskbar appearance export. Existing files are protected. Search redirection and sign-in stay off. |
| `--config FILE` | Use a separate configuration file. |

## Build and stage

From the Rust folder, run `./Build.ps1 test` and `./Stage.ps1 -StyleRoot <portable-Windhawk-folder>`. The build uses the project’s portable Rust toolchain or an available Rust MSVC toolchain, Visual C++ linker, and Windows SDK resource compiler. Set `RC` to the resource compiler’s path if it is installed outside the SDK locations. Caches and release artifacts go under the project’s `bin` folder. The stage script places the executable, notices and path to the style host in `bin/rust-preview`. Add `-CreateShortcuts` to create Desktop and Start menu shortcuts.

Build the per-user installer with `./Installer.ps1 -StyleRoot <prepared-portable-Windhawk-folder> -Compiler <path-to-makensis.exe>`. It outputs `bin/installer/SmartTaskbar-Setup.exe`. The package includes the Windhawk modules with their source and notices. It configures shell paths on the destination PC during installation.

Run `./Prepare-Windhawk.ps1 -OutputDirectory ../bin/portable-style` to prepare the pinned portable host and modules for an installer build. The offline Windhawk setup supplies the compiler for SmartTaskbar Popup Anchors. The host ends up in `../bin/portable-style/Windhawk`. Close any running Windhawk instance before preparing a fresh host. The Rust engine reads and restores the prior native profiles. A `Windhawk` folder beside the executable or an explicit `SMARTTASKBAR_STYLE_ROOT` can also select the host. Without a style host the engine controls native taskbar visibility and Windows supplies its appearance.

The optional redirect executable, license, source archive, and preferences are stored under `optional/search` next to the configuration file. See [third-party notices](NOTICES.md).

## Compare performance in CAD

Open the same CAD model and repeat the same viewport actions for each capture. Use the Pause or Resume control to select the state, then close Settings before capturing. Run these commands from the Rust folder while FreeCAD or SOLIDWORKS is open.

```powershell
./Measure-Cad.ps1 -Condition Enabled -Workflow "Same model and viewport rotation" -Seconds 45 -Trace
./Measure-Cad.ps1 -Condition Paused -Workflow "Same model and viewport rotation" -Seconds 45 -Trace
./Measure-Cad.ps1 -EnabledResult <enabled.json> -PausedResult <paused.json>
```

The JSON comparison reports CPU time and private memory for SmartTaskbar, its styling host, Explorer, Desktop Window Manager and CAD. Open the optional ETL files in Windows Performance Analyzer to compare CPU scheduling and desktop composition. Run the trace captures from an elevated PowerShell window if Windows requires it.

To measure reveal latency separately, run `./Measure-Reveal.ps1 -DisplayNumber 1 -Seconds 30` and repeat the fullscreen edge gesture on that display. The probe reports median and 95th-percentile time from entering the edge to the first visible portion of the taskbar. It samples the pointer without moving it.
