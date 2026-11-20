using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace SmartTaskbar.Appearance;

internal readonly record struct AppearanceTheme(bool Dark, Color Window, Color Card, Color Input, Color Text, Color Muted,
    Color Line, Color Accent, Color AccentText)
{
    public static AppearanceTheme Read()
    {
        if (SystemInformation.HighContrast)
            return new(false, SystemColors.Window, SystemColors.Window, SystemColors.Window, SystemColors.WindowText,
                SystemColors.WindowText, SystemColors.WindowText, SystemColors.Highlight, SystemColors.HighlightText);

        using var theme = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        var dark = theme?.GetValue("SystemUsesLightTheme") is not 1;
        return dark
            ? new(true, Rgb(20, 23, 30), Rgb(28, 33, 42), Rgb(36, 43, 54), Rgb(239, 243, 249),
                Rgb(170, 181, 198), Rgb(59, 69, 85), Rgb(131, 186, 255), Rgb(13, 28, 48))
            : new(false, Rgb(242, 246, 252), Color.White, Rgb(234, 240, 248), Rgb(23, 37, 57),
                Rgb(63, 80, 104), Rgb(197, 209, 224), Rgb(25, 80, 150), Color.White);
    }

    private static Color Rgb(int r, int g, int b) => Color.FromArgb(r, g, b);

    public void ApplyTitleBar(IntPtr window)
    {
        var value = Dark ? 1 : 0;
        DwmSetWindowAttribute(window, 20, ref value, sizeof(int));
    }

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(IntPtr window, int attribute, ref int value, int size);
}
