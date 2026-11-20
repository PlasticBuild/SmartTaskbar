using System.Diagnostics;
using Microsoft.Win32;

namespace SmartTaskbar.Appearance;

internal sealed class SignInSettings(string folder, RegistryKey user)
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string Name = "SmartTaskbarAppearance";
    private const string Preference = "StartAtSignIn";
    private string Command => $"\"{Path.GetFullPath(Path.Combine(folder, "SmartTaskbar.Appearance.exe"))}\" --resume";

    internal bool Enabled
    {
        get
        {
            using var key = user.OpenSubKey(RunKey);
            return string.Equals(key?.GetValue(Name) as string, Command, StringComparison.OrdinalIgnoreCase);
        }
    }

    internal void Save(bool enabled)
    {
        if (enabled && EnginePath() == null)
            throw new InvalidOperationException("SmartTaskbar.exe is missing from the installation. Restore it before enabling sign-in.");
        using var preferences = user.CreateSubKey(IslandMenuWidth.Preferences);
        SetRunEntry(enabled);
        preferences.SetValue(Preference, enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    internal void ApplyLayout(bool styled)
    {
        using var preferences = user.OpenSubKey(IslandMenuWidth.Preferences);
        // Older installations registered the styling host only while a styled layout was active.
        // Once the user chooses the sign-in behavior, that choice applies to every layout.
        if (preferences?.GetValue(Preference) is not int)
            SetRunEntry(styled);
    }

    internal void StartEngine()
    {
        if (Enabled && EnginePath() is { } engine)
            Process.Start(new ProcessStartInfo(engine)
            {
                WorkingDirectory = Path.GetDirectoryName(engine)!,
                UseShellExecute = false,
                CreateNoWindow = true
            })?.Dispose();
    }

    private string? EnginePath()
    {
        var bundled = Path.Combine(folder, "SmartTaskbar.exe");
        var installed = Path.GetFullPath(Path.Combine(folder, "..", "Installed", "SmartTaskbar.exe"));
        return File.Exists(bundled) ? bundled : File.Exists(installed) ? installed : null;
    }

    private void SetRunEntry(bool enabled)
    {
        using var key = user.CreateSubKey(RunKey);
        var previous = key.GetValue(Name) as string;
        if (string.Equals(previous, Command, StringComparison.OrdinalIgnoreCase))
        {
            if (!enabled) key.DeleteValue(Name, false);
        }
        else if (enabled)
        {
            if (previous != null)
                throw new InvalidOperationException("Another copy of SmartTaskbar Appearance is configured to run at sign-in.");
            key.SetValue(Name, Command, RegistryValueKind.String);
        }
    }
}
