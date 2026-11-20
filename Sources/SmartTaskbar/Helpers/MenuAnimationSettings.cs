using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace SmartTaskbar;

internal static class MenuAnimationSettings
{
    private const string SettingsKey = @"Software\SmartTaskbar\Desktop";
    private const string Name = "MenuAnimations";

    public static bool Read()
    {
        using var key = Registry.CurrentUser.OpenSubKey(SettingsKey);
        return key?.GetValue(Name) switch
        {
            0 => false,
            1 => true,
            _ => SystemParametersInfo(0x1042, 0, out var enabled, 0) ? enabled != 0 : SystemInformation.IsMenuAnimationEnabled
        };
    }

    public static void Save(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(SettingsKey);
        key.SetValue(Name, enabled ? 1 : 0, RegistryValueKind.DWord);
    }

    [DllImport("user32.dll", EntryPoint = "SystemParametersInfoW")]
    private static extern bool SystemParametersInfo(uint action, uint parameter, out int value, uint flags);
}
