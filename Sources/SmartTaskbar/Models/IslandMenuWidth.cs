using System.Drawing;

namespace SmartTaskbar;

internal static class IslandMenuWidth
{
    internal const string Preferences = @"Software\SmartTaskbar\Desktop";
    internal const string CompanionSetting = "IslandWidthCompanion";
    internal const string CornerRadiusSetting = "IslandCornerRadius";
    internal const string Argument = "--sync-island-start";
    internal const string FlyoutOpenEvent = @"Local\SmartTaskbar.IslandFlyoutOpen";
    internal const int Maximum = 760;
    internal const int SideInset = 12;

    internal static int SearchInset(Rectangle bar, Rectangle work, uint dpi)
    {
        var overlap = Math.Clamp(work.Bottom - bar.Top, 0, bar.Height);
        // Search includes its own lower shadow gutter inside the native host.
        var depth = (int)Math.Round(overlap * 96d / (dpi == 0 ? 96u : dpi));
        return Math.Max(0, depth - 18);
    }

    internal static int TrayShift(Rectangle island, Rectangle work, uint dpi)
    {
        var scale = dpi == 0 ? 96u : dpi;
        var inset = (int)Math.Round((work.Right - island.Right) * 96d / scale);
        var available = (int)((long)work.Width * 96 / scale);
        // The wider of the two native tray wrappers is 384 DIPs, including its shadow.
        return Math.Clamp(SideInset - inset, Math.Min(0, 384 - available), 0);
    }

    internal static int Measure(int islandPixels, int monitorPixels, uint dpi)
    {
        var scale = dpi == 0 ? 96u : dpi;
        var available = Math.Max(1, (int)((long)monitorPixels * 96 / scale));
        var desired = (int)Math.Round(islandPixels * 96d / scale, MidpointRounding.AwayFromZero);
        return Math.Clamp(desired, Math.Min(420, available), Math.Min(Maximum, available));
    }

    internal static int StartFaceLeft(Rectangle wrapper, Rectangle island, int width, int dpi)
    {
        var before = island.Left - wrapper.Left;
        var after = wrapper.Right - island.Right;
        // Side presets use a 12-DIP spacer. The native Start frame aligns its outside
        // edge there even if its minimum or maximum makes it wider or shorter than the island.
        var tolerance = Math.Max(2, (2 * dpi + 48) / 96);
        var inset = (SideInset * dpi + 48) / 96;
        var left = island.Left + (island.Width - width) / 2;
        if (Math.Abs(before - inset) <= tolerance && after - before > tolerance)
            left = island.Left;
        else if (Math.Abs(after - inset) <= tolerance && before - after > tolerance)
            left = island.Right - width;
        return Math.Clamp(left, wrapper.Left, wrapper.Right - width);
    }
}
