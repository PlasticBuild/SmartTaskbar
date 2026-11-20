using System.Runtime.InteropServices;

namespace SmartTaskbar.Appearance;

internal static class PrimaryIslandWidth
{
    internal static int Read()
    {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try { return ReadPhysicalWidth(); }
        finally { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
    }

    private static int ReadPhysicalWidth()
    {
        var tray = FindWindow("Shell_TrayWnd", null);
        if (tray == IntPtr.Zero || !GetWindowRect(tray, out var window))
            return 592;
        var dpi = GetDpiForWindow(tray);
        var width = window.Right - window.Left;
        var height = window.Bottom - window.Top;
        var monitorWidth = Screen.FromHandle(tray).Bounds.Width;
        if (width > height && GetWindowRgnBox(tray, out var region) > 1 && region.Right > region.Left &&
            region.Right - region.Left < width && region.Bottom - region.Top == height)
            return IslandMenuWidth.Measure(region.Right - region.Left, monitorWidth, dpi);
        return IslandMenuWidth.Measure((int)(592 * (dpi == 0 ? 96 : dpi) / 96), monitorWidth, dpi);
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
