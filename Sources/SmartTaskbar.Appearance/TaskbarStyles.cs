using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json;
using Microsoft.Win32;

namespace SmartTaskbar.Appearance;

internal enum TaskbarLayout
{
    Windows,
    FullWidth,
    Island,
    Custom
}

internal enum IslandAlignment { Center, Left, Right }

internal readonly record struct AppearanceOptions(TaskbarLayout Layout, int Radius = 8, int Transparency = 40,
    int HoverLift = AppearanceOptions.DefaultHoverLift, int ClickDepth = AppearanceOptions.DefaultClickDepth)
{
    public IslandAlignment Alignment { get; init; }
    public const int MaxRadius = 24;
    public const int MaxTransparency = 85;
    public const int DefaultHoverLift = 3;
    public const int DefaultClickDepth = 2;
    public const int MaxHoverLift = 7;
    public const int MaxClickDepth = 5;
}

internal sealed class TaskbarStyles(string folder)
{
    private const string ModId = "windows-11-taskbar-styler";
    private const string ModVersion = "1.11.2";
    internal SignInSettings SignIn { get; } = new(folder, Registry.CurrentUser);
    private readonly string _portable = Path.Combine(folder, "Windhawk");
    private string Cli => Path.Combine(_portable, "windhawk-cli.exe");
    private string Host => Path.Combine(_portable, "windhawk.exe");
    private static readonly JsonElement WindowsPreset = JsonSerializer.SerializeToElement(new Dictionary<string, string>
    {
        ["theme"] = "",
        ["clickThroughTaskbar"] = "0",
        ["xamlDiagnosticsHandling"] = "alert"
    });

    private sealed record Profile(bool Enabled, JsonElement Settings)
    {
        public AppearanceOptions Options
        {
            get
            {
                if (!Enabled || JsonElement.DeepEquals(Settings, WindowsPreset))
                    return new(TaskbarLayout.Windows);
                var radius = ReadNumber(Settings, "styleConstants[0]", "SmartTaskbarRadius=", 8);
                var transparency = ReadNumber(Settings, "styleConstants[1]", "SmartTaskbarTransparency=", 40);
                var hover = ReadConstant("SmartTaskbarHoverLift=") ?? AppearanceOptions.DefaultHoverLift;
                var click = ReadConstant("SmartTaskbarClickDepth=") ?? AppearanceOptions.DefaultClickDepth;
                var alignment = (IslandAlignment)(ReadConstant("SmartTaskbarAlignment=") ?? 0);
                foreach (var layout in new[] { TaskbarLayout.FullWidth, TaskbarLayout.Island })
                {
                    var options = new AppearanceOptions(layout, radius, transparency, hover, click)
                    {
                        Alignment = layout == TaskbarLayout.Island ? alignment : IslandAlignment.Center
                    };
                    if (JsonElement.DeepEquals(Settings, BuildPreset(options)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: true, guardVerticalRotation: false)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: false)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: false, joinedMenus: false)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: false, includeMenus: false)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: false, includeClickThrough: false, includeMenus: false)) ||
                        JsonElement.DeepEquals(Settings, BuildProfile(options, verticalIsland: false, includeMotion: false,
                            includeClickThrough: false, includeMenus: false)))
                        return options;
                }
                return new(TaskbarLayout.Custom);
            }
        }

        private int? ReadConstant(string prefix)
        {
            foreach (var entry in Settings.EnumerateObject())
            {
                if (entry.Name.StartsWith("styleConstants[", StringComparison.Ordinal) &&
                    entry.Value.GetString() is { } value && value.StartsWith(prefix, StringComparison.Ordinal) &&
                    int.TryParse(value.AsSpan(prefix.Length), CultureInfo.InvariantCulture, out var number))
                    return number;
            }
            return null;
        }
    }

    private sealed record ShellProfile(string Id, Profile Profile);

    public async Task<(AppearanceOptions Options, bool Running, bool CurrentPreset)> ReadAsync()
    {
        var (profile, shellProfiles) = await ReadProfilesAsync();
        var options = profile.Options;
        var current = options.Layout is TaskbarLayout.FullWidth or TaskbarLayout.Island &&
            JsonElement.DeepEquals(profile.Settings, BuildPreset(options)) &&
            shellProfiles.All(shell => shell.Profile.Enabled &&
                ShellFlyoutStyles.IsCurrent(shell.Id, options, shell.Profile.Settings));
        RefreshWidthCompanion(options);
        return (options, IsHostRunning(rejectOtherInstall: false),
            current);
    }

    public async Task ApplyAsync(AppearanceOptions options)
    {
        using var writeLock = await AcquireWriteLockAsync();
        var (previous, shellProfiles) = await ReadProfilesAsync();
        var styled = options.Layout != TaskbarLayout.Windows;

        if (styled)
        {
            await CheckScopeAsync();
            IsHostRunning(rejectOtherInstall: true);
        }
        try
        {
            // A neutral preset restores the shell's original properties without unloading the styler.
            await ChangePresetAsync(ModId, previous, styled ? BuildPreset(options) : WindowsPreset, styled);
            foreach (var shell in shellProfiles)
                await ChangePresetAsync(shell.Id, shell.Profile, ShellFlyoutStyles.Build(shell.Id, options), styled);
            if (styled)
                await StartHostAsync();
            var (updated, _) = await ReadProfilesAsync();
            if (updated.Options != options)
                throw new InvalidOperationException("Windows did not retain the requested profile.");
            SignIn.ApplyLayout(styled);
            RefreshWidthCompanion(options, applied: true);
        }
        catch (Exception error) when (IsServiceError(error))
        {
            try
            {
                await WritePresetAsync(previous.Enabled ? previous.Settings : WindowsPreset);
                foreach (var shell in shellProfiles)
                    await WritePresetAsync(shell.Profile.Enabled ? shell.Profile.Settings :
                        ShellFlyoutStyles.Build(shell.Id, new(TaskbarLayout.Windows)), shell.Id);
            }
            catch (Exception rollback) when (IsServiceError(rollback))
            {
                throw new InvalidOperationException("The layout change failed. Use Restore Windows look to reset the styling.", rollback);
            }
            throw;
        }
    }

    internal async Task<bool> SyncStartWidthAsync(int width, bool updatePosition, int? trayShift, int searchInset)
    {
        if (width is < 0 or > IslandMenuWidth.Maximum)
            return true;
        using var writeLock = await AcquireWriteLockAsync();
        var (current, profiles) = await ReadProfilesAsync();
        var options = current.Options;
        if (options.Layout != TaskbarLayout.Island)
            return true;
        // A native position change reloads shell settings and can move an open panel.
        // The engine signals this throughout the menu interaction. Retry once it closes.
        EventWaitHandle.TryOpenExisting(IslandMenuWidth.FlyoutOpenEvent, out var menu);
        using (menu)
        {
            foreach (var shell in profiles)
            {
                if (!shell.Profile.Enabled) continue;
                var settings = shell.Profile.Settings;
                var desired = shell.Id switch
                {
                    ShellFlyoutStyles.StartStyler when (width > 0 ? width : ShellFlyoutStyles.ReadStartWidth(settings)) is int startWidth =>
                        ShellFlyoutStyles.BuildStart(options, startWidth, searchInset),
                    ShellFlyoutStyles.Positions when updatePosition => ShellFlyoutStyles.BuildPositions(options, trayShift),
                    _ => default
                };
                if (desired.ValueKind == JsonValueKind.Undefined || JsonElement.DeepEquals(desired, shell.Profile.Settings))
                    continue;
                var managed = ShellFlyoutStyles.IsCurrent(shell.Id, options, settings) || shell.Id switch
                {
                    ShellFlyoutStyles.Positions => JsonElement.DeepEquals(settings,
                        ShellFlyoutStyles.BuildPositions(options, null, legacy: true)),
                    ShellFlyoutStyles.StartStyler => ShellFlyoutStyles.IsCurrent(shell.Id,
                        options with { Alignment = IslandAlignment.Center }, settings),
                    _ => false
                };
                if (!managed)
                    continue;
                if (menu?.WaitOne(0) == true)
                    return false;
                await WritePresetAsync(desired, shell.Id);
            }
            return true;
        }
    }

    private async Task<FileStream> AcquireWriteLockAsync()
    {
        var path = Path.Combine(folder, ".appearance-write.lock");
        var deadline = Environment.TickCount64 + 15_000;
        while (true)
        {
            try { return new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None, 1,
                FileOptions.DeleteOnClose); }
            catch (IOException) when (Environment.TickCount64 < deadline) { await Task.Delay(120); }
        }
    }

    public async Task ResumeAsync()
    {
        SignIn.StartEngine();
        using var writeLock = await AcquireWriteLockAsync();
        var (previous, _) = await ReadProfilesAsync();
        var options = previous.Options;
        RefreshWidthCompanion(options);
        if (options.Layout is not (TaskbarLayout.FullWidth or TaskbarLayout.Island))
            return;

        await CheckScopeAsync();
        // Match only our previously shipped side layouts before migrating their orientation rules.
        if (options.Layout == TaskbarLayout.Island &&
            (JsonElement.DeepEquals(previous.Settings, BuildProfile(options, verticalIsland: false)) ||
             JsonElement.DeepEquals(previous.Settings, BuildProfile(options, verticalIsland: true, guardVerticalRotation: false))))
            await WritePresetAsync(BuildPreset(options));
        await StartHostAsync();
    }

    private void RefreshWidthCompanion(AppearanceOptions options, bool applied = false)
    {
        var picker = Path.GetFullPath(Path.Combine(folder, "SmartTaskbar.Appearance.exe"));
        using var current = Registry.CurrentUser.OpenSubKey(IslandMenuWidth.Preferences);
        var registered = current?.GetValue(IslandMenuWidth.CompanionSetting) as string;
        var ours = string.Equals(registered, picker, StringComparison.OrdinalIgnoreCase);
        var wanted = options.Layout == TaskbarLayout.Island && (ours || SignIn.Enabled || applied);
        var radius = Math.Clamp(options.Radius, 0, AppearanceOptions.MaxRadius);
        if (wanted == ours && (!wanted || Equals(current?.GetValue(IslandMenuWidth.CornerRadiusSetting), radius)))
            return;
        using var settings = Registry.CurrentUser.CreateSubKey(IslandMenuWidth.Preferences);
        if (wanted)
        {
            settings.SetValue(IslandMenuWidth.CompanionSetting, picker);
            settings.SetValue(IslandMenuWidth.CornerRadiusSetting, radius, RegistryValueKind.DWord);
        }
        else if (ours)
        {
            settings.DeleteValue(IslandMenuWidth.CompanionSetting, false);
            settings.DeleteValue(IslandMenuWidth.CornerRadiusSetting, false);
        }
    }

    private async Task<(Profile Taskbar, List<ShellProfile> Flyouts)> ReadProfilesAsync()
    {
        var data = await RunCliAsync(["mod", "list"]);
        Profile? taskbar = null;
        var flyouts = new List<ShellProfile>();
        foreach (var item in data.GetProperty("mods").EnumerateArray())
        {
            var id = item.GetProperty("id").GetString();
            var version = item.GetProperty("version").GetString();
            if (id is null || (id == ModId ? version != ModVersion : !ShellFlyoutStyles.Matches(id, version)))
                throw new InvalidOperationException("The portable styling components have been changed.");

            var settings = await RunCliAsync(["mod", "settings", "get", id]);
            var profile = new Profile(item.GetProperty("enabled").GetBoolean(), settings.GetProperty("settings"));
            if (id == ModId)
                taskbar = profile;
            else
                flyouts.Add(new ShellProfile(id, profile));
        }
        return (taskbar ?? throw new InvalidOperationException("The taskbar styling component is missing."), flyouts);
    }

    private async Task ChangePresetAsync(string id, Profile previous, JsonElement preset, bool enabled)
    {
        if (!previous.Enabled && !enabled)
            return;
        if (!JsonElement.DeepEquals(previous.Settings, preset))
            await WritePresetAsync(preset, id);
        if (!previous.Enabled)
            await RunCliAsync(["mod", "enable", id]);
    }

    private async Task CheckScopeAsync()
    {
        var data = await RunCliAsync(["app", "settings", "get"]);
        var settings = data.GetProperty("settings");
        var engine = settings.GetProperty("engine");
        var include = engine.GetProperty("include");
        var exclude = engine.GetProperty("exclude");
        var explorer = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe");
        var paths = include.EnumerateArray().Select(value => value.GetString()).ToArray();
        if (settings.GetProperty("safeMode").GetBoolean() ||
            !paths.Contains(explorer, StringComparer.OrdinalIgnoreCase) || paths.Any(path => !IsKnownShellPath(path, explorer)) ||
            exclude.GetArrayLength() != 1 || exclude[0].GetString() != "*" ||
            engine.GetProperty("injectIntoCriticalProcesses").GetBoolean() ||
            engine.GetProperty("injectIntoIncompatiblePrograms").GetBoolean() ||
            engine.GetProperty("injectIntoGames").GetBoolean())
            throw new InvalidOperationException("The portable styling engine must be scoped to Windows Explorer and its menu hosts.");
    }

    private static bool IsKnownShellPath(string? path, string explorer)
    {
        if (string.Equals(path, explorer, StringComparison.OrdinalIgnoreCase))
            return true;
        if (string.Equals(path, Path.Combine(Environment.SystemDirectory, "ShellHost.exe"), StringComparison.OrdinalIgnoreCase))
            return true;

        var systemApps = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "SystemApps") +
            Path.DirectorySeparatorChar;
        return path is not null && path.StartsWith(systemApps, StringComparison.OrdinalIgnoreCase) &&
            Path.GetFullPath(path).StartsWith(systemApps, StringComparison.OrdinalIgnoreCase) &&
            Path.GetFileName(path) is "StartMenuExperienceHost.exe" or "SearchHost.exe" or "SearchApp.exe" or "ShellExperienceHost.exe";
    }

    private async Task StartHostAsync()
    {
        if (IsHostRunning(rejectOtherInstall: true))
            return;

        using var process = Process.Start(new ProcessStartInfo(Host, "-tray-only")
        {
            WorkingDirectory = _portable,
            UseShellExecute = false,
            CreateNoWindow = true
        }) ?? throw new InvalidOperationException("Could not start the taskbar styling engine.");

        await Task.Delay(250);
        if (process.HasExited && !IsHostRunning(rejectOtherInstall: true))
            throw new InvalidOperationException("The taskbar styling engine failed to start.");
    }

    private bool IsHostRunning(bool rejectOtherInstall)
    {
        using var current = Process.GetCurrentProcess();
        var running = false;
        var processes = Process.GetProcessesByName("windhawk");
        try
        {
            foreach (var process in processes)
            {
                if (process.HasExited || process.SessionId != current.SessionId)
                    continue;

                string? path;
                try { path = process.MainModule?.FileName; }
                catch (Win32Exception) { path = null; }
                if (string.Equals(path, Host, StringComparison.OrdinalIgnoreCase))
                    running = true;
                else if (rejectOtherInstall)
                    throw new InvalidOperationException("Close the other Windhawk instance before applying a SmartTaskbar layout.");
            }
            return running;
        }
        finally
        {
            foreach (var process in processes)
                process.Dispose();
        }
    }

    private Task<JsonElement> WritePresetAsync(JsonElement preset, string id = ModId)
        => RunCliAsync(["mod", "settings", "write", id, "-"], preset.GetRawText());

    private static int ReadNumber(JsonElement settings, string key, string prefix, int fallback)
    {
        if (settings.TryGetProperty(key, out var property) && property.GetString() is { } value &&
            value.StartsWith(prefix, StringComparison.Ordinal) &&
            int.TryParse(value.AsSpan(prefix.Length), CultureInfo.InvariantCulture, out var number))
            return number;
        return fallback;
    }

    private static JsonElement BuildPreset(AppearanceOptions options, bool includeMotion = true, bool includeClickThrough = true,
        bool includeMenus = true, bool joinedMenus = true)
        => BuildProfile(options, true, includeMotion, includeClickThrough, includeMenus, joinedMenus);

    private static JsonElement BuildProfile(AppearanceOptions options, bool verticalIsland,
        bool includeMotion = true, bool includeClickThrough = true, bool includeMenus = true, bool joinedMenus = true,
        bool guardVerticalRotation = true)
    {
        if (options.Layout is not (TaskbarLayout.FullWidth or TaskbarLayout.Island))
            throw new ArgumentOutOfRangeException(nameof(options));

        using var stream = typeof(TaskbarStyles).Assembly.GetManifestResourceStream($"SmartTaskbar.Appearance.Profiles.{options.Layout}.json")
            ?? throw new InvalidOperationException("The selected layout file is missing.");
        using var reader = new StreamReader(stream);
        var radius = Math.Clamp(options.Radius, 0, AppearanceOptions.MaxRadius);
        var transparency = Math.Clamp(options.Transparency, 0, AppearanceOptions.MaxTransparency);
        var sideInset = IslandMenuWidth.SideInset.ToString(CultureInfo.InvariantCulture);
        var preset = reader.ReadToEnd()
            .Replace("__LEADING__", options.Alignment == IslandAlignment.Left ? sideInset : "*", StringComparison.Ordinal)
            .Replace("__TRAILING__", options.Alignment == IslandAlignment.Right ? sideInset : "*", StringComparison.Ordinal)
            .Replace("__EMBEDDED_INSET__", options.Alignment == IslandAlignment.Center ? "max((BarWidth-$Width)/2,0)" : "0", StringComparison.Ordinal)
            .Replace("__RADIUS__", radius.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("__TRANSPARENCY__", transparency.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("__TINT_OPACITY__", (1 - transparency / 100d).ToString("0.00", CultureInfo.InvariantCulture), StringComparison.Ordinal);
        var settings = JsonSerializer.Deserialize<Dictionary<string, string>>(preset)!;
        if (verticalIsland && options.Layout == TaskbarLayout.Island)
        {
            using var vertical = typeof(TaskbarStyles).Assembly.GetManifestResourceStream(
                "SmartTaskbar.Appearance.Profiles.IslandVertical.json")!;
            foreach (var (key, value) in JsonSerializer.Deserialize<Dictionary<string, string>>(vertical)!)
                settings[key] = value.Replace("__RADIUS__", radius.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
                    .Replace("__TINT_OPACITY__", (1 - transparency / 100d).ToString("0.00", CultureInfo.InvariantCulture), StringComparison.Ordinal);
            if (guardVerticalRotation)
            {
                // Explorer can measure the new side host before its docking state updates. A horizontal
                // desired width can then become the thickness of the primary taskbar for the entire session.
                settings["styleConstants[2]"] = "Horizontal=(DockEdge==`left`?0:(DockEdge==`right`?0:(HostWidth==``?1:(HostHeight==``?1:HostWidth>=HostHeight))))";
                settings["controlStyles[2].styles[1]"] = "Tag@DockedLeft=left";
                settings["controlStyles[2].styles[2]"] = "Tag@DockedRight=right";
                settings["controlStyles[12].target"] = "Taskbar.ExperienceToggleButton[AutomationProperties.AutomationId=StartButton]";
                settings["controlStyles[12].styles[0]"] = "ActualWidth=>NativeButtonWidth";
                settings["controlStyles[13].target"] = ":root > ScrollViewer";
                settings["controlStyles[13].styles[0]"] = "ActualWidth=>HostWidth";
                settings["controlStyles[13].styles[1]"] = "ActualHeight=>HostHeight";
                settings["controlStyles[1].styles[3]"] = "MaxWidth={{$Horizontal?skip():(NativeButtonWidth==``?skip():(NativeButtonWidth>0?NativeButtonWidth:skip()))}}";
                settings["controlStyles[1].styles[4]"] = "HorizontalAlignment={{DockEdge==`right`?`Right`:(DockEdge==`left`?`Left`:skip())}}";
            }
        }
        if (includeClickThrough && options.Layout == TaskbarLayout.Island)
            settings["clickThroughTaskbar"] = "1";
        if (includeMotion)
            AddIconMotion(settings, options);
        if (options.Layout == TaskbarLayout.Island && options.Alignment != IslandAlignment.Center)
        {
            var index = settings.Keys.Count(key => key.StartsWith("styleConstants[", StringComparison.Ordinal));
            settings[$"styleConstants[{index}]"] = $"SmartTaskbarAlignment={(int)options.Alignment}";
        }
        if (includeMenus)
            ShellFlyoutStyles.AddTaskbarMenus(settings, options, joinedMenus);
        return JsonSerializer.SerializeToElement(settings);
    }

    private static void AddIconMotion(Dictionary<string, string> settings, AppearanceOptions options)
    {
        var hover = Math.Clamp(options.HoverLift, 0, AppearanceOptions.MaxHoverLift);
        var click = Math.Clamp(options.ClickDepth, 0, AppearanceOptions.MaxClickDepth);
        var constant = settings.Keys.Count(key => key.StartsWith("styleConstants[", StringComparison.Ordinal));
        settings[$"styleConstants[{constant}]"] = $"SmartTaskbarHoverLift={hover}";
        settings[$"styleConstants[{constant + 1}]"] = $"SmartTaskbarClickDepth={click}";
        if (hover == 0 && click == 0)
            return;

        var target = settings.Keys.Count(key => key.StartsWith("controlStyles[", StringComparison.Ordinal) &&
            key.EndsWith(".target", StringComparison.Ordinal));
        var group = $"controlStyles[{target}]";
        // Transform the image so the shell keeps its original button bounds and click handling.
        settings[group + ".target"] =
            "Taskbar.TaskListButton > Grid#IconPanel@CommonStates > Image#Icon, " +
            "Taskbar.TaskListButton > Taskbar.TaskListLabeledButtonPanel#IconPanel@CommonStates > Image#Icon";
        var styles = new List<string>
        {
            "RenderTransformOrigin=0.5,0.5"
        };

        var hoverScale = 1 + hover * 0.035;
        var pressScale = click == 0 ? hoverScale : 1 - click * 0.03;
        foreach (var state in new[] { "Inactive", "Active", "MultiWindow", "RequestingAttention", "RequestingAttentionMulti" })
        {
            AddTransform(state + "PointerOver", hoverScale, -hover);
            AddTransform(state + "Pressed", pressScale, click == 0 ? -hover : click);
        }
        for (var index = 0; index < styles.Count; index++)
            settings[$"{group}.styles[{index}]"] = styles[index];

        void AddTransform(string state, double scale, int y)
        {
            styles.Add(FormattableString.Invariant(
                $"RenderTransform@{state}:=<CompositeTransform ScaleX=\"{scale:0.###}\" ScaleY=\"{scale:0.###}\" TranslateY=\"{y}\" />"));
        }
    }

    private async Task<JsonElement> RunCliAsync(string[] arguments, string? input = null)
    {
        if (!File.Exists(Host) || !File.Exists(Cli))
            throw new InvalidOperationException("The portable styling component is missing from the Windhawk folder.");

        var output = await RunAsync(Cli, ["--app-root", _portable, "--json", .. arguments], input);
        using var document = JsonDocument.Parse(output);
        return document.RootElement.GetProperty("data").Clone();
    }

    private async Task<string> RunAsync(string path, string[] args, string? input = null)
    {
        var info = new ProcessStartInfo(path)
        {
            WorkingDirectory = _portable,
            CreateNoWindow = true,
            UseShellExecute = false,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            RedirectStandardInput = input is not null
        };
        if (input is not null)
            info.StandardInputEncoding = new UTF8Encoding(false);
        foreach (var arg in args)
            info.ArgumentList.Add(arg);

        using var process = Process.Start(info) ?? throw new InvalidOperationException("Could not run the taskbar styling component.");
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(20));
        var output = process.StandardOutput.ReadToEndAsync(timeout.Token);
        var errors = process.StandardError.ReadToEndAsync(timeout.Token);
        try
        {
            if (input is not null)
            {
                await process.StandardInput.WriteAsync(input.AsMemory(), timeout.Token);
                process.StandardInput.Close();
            }
            await process.WaitForExitAsync(timeout.Token);
            await Task.WhenAll(output, errors);
        }
        catch (OperationCanceledException)
        {
            if (!process.HasExited)
                process.Kill();
            throw new InvalidOperationException("The taskbar styling component timed out.");
        }

        if (process.ExitCode != 0)
        {
            using var reply = TryParseReply(output.Result);
            var detail = reply?.RootElement.GetProperty("error").GetProperty("message").GetString();
            throw new InvalidOperationException(detail ?? $"The taskbar styling component returned error {process.ExitCode}.");
        }
        return output.Result;
    }

    private static JsonDocument? TryParseReply(string output)
    {
        try { return JsonDocument.Parse(output); }
        catch (JsonException) { return null; }
    }

    internal static bool IsServiceError(Exception error) => error is IOException or Win32Exception or
        UnauthorizedAccessException or InvalidOperationException or JsonException or KeyNotFoundException;
}
