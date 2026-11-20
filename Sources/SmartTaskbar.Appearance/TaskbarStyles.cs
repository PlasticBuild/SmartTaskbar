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

internal readonly record struct AppearanceOptions(TaskbarLayout Layout, int Radius = 8, int Transparency = 40)
{
    public const int MaxRadius = 24;
    public const int MaxTransparency = 85;
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
                foreach (var layout in new[] { TaskbarLayout.FullWidth, TaskbarLayout.Island })
                {
                    var options = new AppearanceOptions(layout, radius, transparency);
                    if (JsonElement.DeepEquals(Settings, BuildPreset(options)))
                        return options;
                }
                return new(TaskbarLayout.Custom);
            }
        }
    }

    public async Task<(AppearanceOptions Options, bool Running)> ReadAsync()
    {
        var profile = await ReadProfileAsync();
        return (profile.Options, IsHostRunning(rejectOtherInstall: false));
    }

    public async Task ApplyAsync(AppearanceOptions options)
    {
        var previous = await ReadProfileAsync();
        if (options.Layout == TaskbarLayout.Windows)
        {
            // Let the mod restore XAML properties live. Unloading it mid-session can remove Explorer's tray UI.
            if (previous.Enabled)
                await WritePresetAsync(WindowsPreset);
            SetStartup(false);
            return;
        }

        var preset = BuildPreset(options);
        await CheckScopeAsync();
        IsHostRunning(rejectOtherInstall: true);
        try
        {
            await WritePresetAsync(preset);
            if (!previous.Enabled)
                await RunCliAsync(["mod", "enable", ModId]);
            await StartHostAsync();
            var updated = await ReadProfileAsync();
            if (updated.Options != options)
                throw new InvalidOperationException("Windows did not retain the requested profile.");
            SetStartup(true);
        }
        catch (Exception error) when (IsServiceError(error))
        {
            try
            {
                await WritePresetAsync(previous.Enabled ? previous.Settings : WindowsPreset);
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
        var previous = await ReadProfileAsync();
        if (previous.Options.Layout is not (TaskbarLayout.FullWidth or TaskbarLayout.Island))
            return;

        await CheckScopeAsync();
        await StartHostAsync();
    }

    private async Task<Profile> ReadProfileAsync()
    {
        var data = await RunCliAsync(["mod", "list"]);
        var mods = data.GetProperty("mods");
        if (mods.GetArrayLength() != 1 || mods[0].GetProperty("id").GetString() != ModId ||
            mods[0].GetProperty("version").GetString() != ModVersion)
            throw new InvalidOperationException("The portable taskbar styling component is missing or has been changed.");

        var settings = await RunCliAsync(["mod", "settings", "get", ModId]);
        return new Profile(mods[0].GetProperty("enabled").GetBoolean(), settings.GetProperty("settings"));
    }

    private async Task CheckScopeAsync()
    {
        var data = await RunCliAsync(["app", "settings", "get"]);
        var settings = data.GetProperty("settings");
        var engine = settings.GetProperty("engine");
        var include = engine.GetProperty("include");
        var exclude = engine.GetProperty("exclude");
        var explorer = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe");
        if (settings.GetProperty("safeMode").GetBoolean() ||
            include.GetArrayLength() != 1 ||
            !string.Equals(include[0].GetString(), explorer, StringComparison.OrdinalIgnoreCase) ||
            exclude.GetArrayLength() != 1 || exclude[0].GetString() != "*" ||
            engine.GetProperty("injectIntoCriticalProcesses").GetBoolean() ||
            engine.GetProperty("injectIntoIncompatiblePrograms").GetBoolean() ||
            engine.GetProperty("injectIntoGames").GetBoolean())
            throw new InvalidOperationException("The portable styling engine must be scoped to Windows Explorer.");
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

    private Task<JsonElement> WritePresetAsync(JsonElement preset)
        => RunCliAsync(["mod", "settings", "write", ModId, "-"], preset.GetRawText());

    private static int ReadNumber(JsonElement settings, string key, string prefix, int fallback)
    {
        if (settings.TryGetProperty(key, out var property) && property.GetString() is { } value &&
            value.StartsWith(prefix, StringComparison.Ordinal) &&
            int.TryParse(value.AsSpan(prefix.Length), CultureInfo.InvariantCulture, out var number))
            return number;
        return fallback;
    }

    private static JsonElement BuildPreset(AppearanceOptions options)
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
        using var json = JsonDocument.Parse(preset);
        return json.RootElement.Clone();
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
