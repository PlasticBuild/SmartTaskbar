using Microsoft.Win32;

namespace SmartTaskbar;

internal static class RevealZoneSettings
{
    private const string SettingsKey = @"Software\SmartTaskbar\Desktop";
    private const string Name = "RevealZone";

    public static RevealZone Read()
    {
        using var key = Registry.CurrentUser.OpenSubKey(SettingsKey);
        return key?.GetValue(Name) switch
        {
            nameof(RevealZone.CenterHalf) => RevealZone.CenterHalf,
            nameof(RevealZone.VisibleTaskbar) => RevealZone.VisibleTaskbar,
            _ => RevealZone.EntireEdge
        };
    }

    public static void Save(RevealZone zone)
    {
        using var key = Registry.CurrentUser.CreateSubKey(SettingsKey);
        key.SetValue(Name, zone.ToString());
    }
}
