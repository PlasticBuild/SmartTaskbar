# SmartTaskbar for Windows 11 in Rust

The engine, settings window, recovery process, profile selection and optional integrations are implemented in Rust. Explorer supplies the native taskbar controls, and a private portable Windhawk host applies the appearance to Windows shell surfaces.

Run `SmartTaskbar-Setup.exe` on 64-bit Intel or AMD Windows 11. Setup installs for the current user, creates one **SmartTaskbar** entry in Start and Installed apps, and can add a desktop shortcut. Rerunning Setup upgrades that installation and keeps its preferences. Open **SmartTaskbar** in Start to change appearance, optional features and sign-in behavior.

Open `smarttaskbar-rs.exe` for settings. The preview lets you try the island, full-width bar, pointer effects, fullscreen reveal and menu transition. Select **Apply** to save. Preferences are stored beside the executable in `settings.json`, or in the file selected with `--config`. Turning on sign-in registers that executable for the current Windows user.

The C# and Rust controllers share Explorer. The Rust engine checks for a running C# controller before it takes control. **Ctrl+Alt+Shift+T** restores the taskbar and exits the Rust engine. `smarttaskbar-rs.exe --stop` also requests a graceful exit.

**App icon size** scales pinned and running app icons from 70% to 125% using the same base size on horizontal and vertical taskbars. It updates the preview as you drag. Select **Apply** to use the new size.

Turn on **Show taskbar media player**, choose **Media player side**, and select **Apply**. The player appears when an app starts reporting playback to Windows. Available artwork fills the player background with a blur. The player offers play or pause, previous and next controls, and a scrubber. Seeking and track navigation depend on the app. The player follows the taskbar orientation and fits inline when there is room. On a vertical taskbar, Left chooses the upper end and Right chooses the lower end.

## Commands

| Command | Action |
| --- | --- |
| `--settings` | Open the settings window. This is the default. |
| `--run` | Run the controller for the current Windows session. |
| `--stop` | Restore the taskbar and stop the Rust controller. |
| `--inspect` | Read the current monitor and taskbar geometry. |
| `--check` | Validate the settings and generate shell profiles entirely in memory. |
| `--import-appearance PROFILE --config FILE` | Create a new settings file from a previous SmartTaskbar appearance export. Existing files are protected. Search redirection and sign-in stay off. |
| `--config FILE` | Use a separate configuration file. |

## Build and stage

From the Rust folder, run `./Build.ps1 test` and `./Stage.ps1 -StyleRoot <portable-Windhawk-folder>`. The build uses the project’s portable Rust toolchain or an available Rust MSVC toolchain and Visual C++ linker. Caches and release artifacts go under the project’s `bin` folder. The stage script places the executable, notices and path to the style host in `bin/rust-preview`. Add `-CreateShortcuts` when installing the Rust version to create separate Desktop and Start menu shortcuts.

Build the per-user installer with `./Installer.ps1 -StyleRoot <prepared-portable-Windhawk-folder> -Compiler <path-to-makensis.exe>`. It outputs `bin/installer/SmartTaskbar-Setup.exe`. The package includes the four versioned Windhawk modules with their source and notices. It configures shell paths on the destination PC during installation.

Use a dedicated portable Windhawk host prepared for this project with taskbar styler 1.11.2, Start menu styler 1.8, notification center styler 1.8 and shell flyout positions 1.3.1. The Rust engine reads and restores the prior native profiles. A `Windhawk` folder beside the executable or an explicit `SMARTTASKBAR_STYLE_ROOT` can also select the host. Without a style host the engine controls native taskbar visibility and Windows supplies its appearance.

Search redirection is off by default. When enabled, the standalone helper and its settings are stored under `optional/search` next to the configuration file. See [third-party notices](NOTICES.md).
