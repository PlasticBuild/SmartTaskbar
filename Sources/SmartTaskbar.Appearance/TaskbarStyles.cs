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

internal readonly record struct AppearanceOptions(TaskbarLayout Layout, int Radius = 8, int Transparency = 40,
    int HoverLift = AppearanceOptions.DefaultHoverLift, int ClickDepth = AppearanceOptions.DefaultClickDepth)
{
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
    private const string StartupKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string StartupName = "SmartTaskbarAppearance";
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
                var hover = ReadMotionSetting("SmartTaskbarHoverLift=") ?? AppearanceOptions.DefaultHoverLift;
                var click = ReadMotionSetting("SmartTaskbarClickDepth=") ?? AppearanceOptions.DefaultClickDepth;
                foreach (var layout in new[] { TaskbarLayout.FullWidth, TaskbarLayout.Island })
                {
                    var options = new AppearanceOptions(layout, radius, transparency, hover, click);
                    if (JsonElement.DeepEquals(Settings, BuildPreset(options)) ||
                        JsonElement.DeepEquals(Settings, BuildPreset(options, includeMenus: false)) ||
                        JsonElement.DeepEquals(Settings, BuildPreset(options, includeClickThrough: false, includeMenus: false)) ||
                        JsonElement.DeepEquals(Settings, BuildPreset(options, includeMotion: false,
                            includeClickThrough: false, includeMenus: false)))
                        return options;
                }
                return new(TaskbarLayout.Custom);
            }
        }

        private int? ReadMotionSetting(string prefix)
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
                JsonElement.DeepEquals(shell.Profile.Settings, ShellFlyoutStyles.Build(shell.Id, options)));
        return (options, IsHostRunning(rejectOtherInstall: false),
            current);
    }

    public async Task ApplyAsync(AppearanceOptions options)
    {
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
            SetStartup(styled);
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

    public async Task ResumeAsync()
    {
        var (previous, _) = await ReadProfilesAsync();
        if (previous.Options.Layout is not (TaskbarLayout.FullWidth or TaskbarLayout.Island))
            return;

        await CheckScopeAsync();
        await StartHostAsync();
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
        bool includeMenus = true)
    {
        if (options.Layout is not (TaskbarLayout.FullWidth or TaskbarLayout.Island))
            throw new ArgumentOutOfRangeException(nameof(options));

        using var stream = typeof(TaskbarStyles).Assembly.GetManifestResourceStream($"SmartTaskbar.Appearance.Profiles.{options.Layout}.json")
            ?? throw new InvalidOperationException("The selected layout file is missing.");
        using var reader = new StreamReader(stream);
        var radius = Math.Clamp(options.Radius, 0, AppearanceOptions.MaxRadius);
        var transparency = Math.Clamp(options.Transparency, 0, AppearanceOptions.MaxTransparency);
        var preset = reader.ReadToEnd()
            .Replace("__RADIUS__", radius.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("__TRANSPARENCY__", transparency.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("__TINT_OPACITY__", (1 - transparency / 100d).ToString("0.00", CultureInfo.InvariantCulture), StringComparison.Ordinal);
        var settings = JsonSerializer.Deserialize<Dictionary<string, string>>(preset)!;
        if (includeClickThrough && options.Layout == TaskbarLayout.Island)
            settings["clickThroughTaskbar"] = "1";
        if (includeMotion)
            AddIconMotion(settings, options);
        if (includeMenus)
            ShellFlyoutStyles.AddTaskbarMenus(settings, options);
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

    private static void SetStartup(bool enabled)
    {
        var command = $"\"{Environment.ProcessPath}\" --resume";
        using var key = Registry.CurrentUser.CreateSubKey(StartupKey);
        var previous = key.GetValue(StartupName) as string;
        if (enabled)
        {
            if (previous is not null && !string.Equals(previous, command, StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Another copy of SmartTaskbar Appearance is already configured to run at sign-in.");
            key.SetValue(StartupName, command);
        }
        else if (string.Equals(previous, command, StringComparison.OrdinalIgnoreCase))
        {
            key.DeleteValue(StartupName, throwOnMissingValue: false);
        }
    }

    internal static bool IsServiceError(Exception error) => error is IOException or Win32Exception or
        UnauthorizedAccessException or InvalidOperationException or JsonException or KeyNotFoundException;
}
