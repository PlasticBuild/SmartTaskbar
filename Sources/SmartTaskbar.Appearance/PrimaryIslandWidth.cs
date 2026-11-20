using System.Runtime.InteropServices;

namespace SmartTaskbar.Appearance;

internal static class PrimaryIslandWidth
{
    internal static (int StartWidth, int? TrayShift, int SearchInset) Read()
    {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try { return ReadPhysicalWidth(); }
        finally { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
    }

    private static (int, int?, int) ReadPhysicalWidth()
    {
        var tray = FindWindow("Shell_TrayWnd", null);
        if (tray == IntPtr.Zero || !GetWindowRect(tray, out var window))
            return (592, null, 0);
        var dpi = GetDpiForWindow(tray);
        var width = window.Right - window.Left;
        var height = window.Bottom - window.Top;
        var screen = Screen.FromHandle(tray);
        var monitorWidth = screen.Bounds.Width;
        if (width > height && GetWindowRgnBox(tray, out var region) > 1 && region.Right > region.Left &&
            region.Right - region.Left < width && region.Bottom - region.Top == height)
        {
            var island = Rectangle.FromLTRB(window.Left + region.Left, window.Top + region.Top,
                window.Left + region.Right, window.Top + region.Bottom);
            var widthDip = IslandMenuWidth.Measure(island.Width, monitorWidth, dpi);
            return window.Bottom == screen.Bounds.Bottom
                ? (widthDip, IslandMenuWidth.TrayShift(island, screen.WorkingArea, dpi),
                    IslandMenuWidth.SearchInset(Rectangle.FromLTRB(window.Left, window.Top, window.Right, window.Bottom), screen.WorkingArea, dpi))
                : (widthDip, null, 0);
        }
        return (IslandMenuWidth.Measure((int)(592 * (dpi == 0 ? 96 : dpi) / 96), monitorWidth, dpi), null, 0);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")]
    private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr FindWindow(string className, string? title);
    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")]
    private static extern int GetWindowRgnBox(IntPtr window, out Rect rect);
    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);
}
