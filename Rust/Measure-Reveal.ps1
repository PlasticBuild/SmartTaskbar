param(
    [Parameter(Mandatory)][ValidateRange(1, 99)][int]$DisplayNumber,
    [ValidateRange(10, 180)][int]$Seconds = 30
)

$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;

public static class SmartTaskbarRevealProbe {
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] struct MonitorInfo {
        public int Size;
        public Rect Display, Work;
        public uint Flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string Name;
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string kind, string title);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect bounds);
    [DllImport("user32.dll")] static extern int GetWindowRgnBox(IntPtr hwnd, out Rect bounds);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);

    public static double[] Record(string display, int seconds) {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try {
            foreach (string kind in new[] {"Shell_TrayWnd", "Shell_SecondaryTrayWnd"}) {
                var hwnd = IntPtr.Zero;
                while ((hwnd = FindWindowEx(IntPtr.Zero, hwnd, kind, null)) != IntPtr.Zero) {
                    var info = new MonitorInfo { Size = Marshal.SizeOf(typeof(MonitorInfo)) };
                    if (GetMonitorInfo(MonitorFromWindow(hwnd, 2), ref info) && info.Name == display)
                        return RecordBar(hwnd, info.Display, seconds);
                }
            }
            throw new InvalidOperationException("Windows does not report a taskbar on that display.");
        } finally {
            if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous);
        }
    }

    static double[] RecordBar(IntPtr hwnd, Rect display, int seconds) {
        Rect bar;
        if (!GetWindowRect(hwnd, out bar)) throw new InvalidOperationException("Could not read the taskbar bounds.");
        int edge = Math.Max(1, (int)GetDpiForWindow(hwnd) * 6 / 96);
        bool horizontal = bar.R - bar.L > bar.B - bar.T;
        bool near = horizontal ? Math.Abs(bar.T - display.T) < Math.Abs(bar.B - display.B)
                               : Math.Abs(bar.L - display.L) < Math.Abs(bar.R - display.R);
        bool oldInside = false, oldHidden = false;
        double entered = -1;
        var samples = new List<double>();
        var watch = Stopwatch.StartNew();
        while (watch.Elapsed.TotalSeconds < seconds) {
            Point cursor;
            Rect region;
            if (!GetCursorPos(out cursor)) break;
            bool hidden = GetWindowRgnBox(hwnd, out region) == 1;
            bool onDisplay = cursor.X >= display.L && cursor.X < display.R &&
                             cursor.Y >= display.T && cursor.Y < display.B;
            int distance = horizontal ? (near ? cursor.Y - display.T : display.B - 1 - cursor.Y)
                                      : (near ? cursor.X - display.L : display.R - 1 - cursor.X);
            bool inside = onDisplay && distance < edge;
            double now = watch.Elapsed.TotalMilliseconds;
            if (!inside) entered = -1;
            else if (!oldInside && oldHidden && hidden) entered = now;
            else if (!hidden && oldHidden && entered >= 0) {
                samples.Add(now - entered);
                entered = -1;
            }
            oldInside = inside;
            oldHidden = hidden;
            Thread.Sleep(12);
        }
        return samples.ToArray();
    }
}
'@

Write-Host "Move into the configured reveal zone on display $DisplayNumber and hold until the bar appears. Move away before repeating."
$samples = @([SmartTaskbarRevealProbe]::Record('\.DISPLAY' + $DisplayNumber, $Seconds) | Sort-Object)
if ($samples.Count -eq 0) { Write-Output 'No completed reveal gestures observed.'; return }
function Get-Percentile([double]$fraction) {
    [Math]::Round($samples[[Math]::Ceiling($samples.Count * $fraction) - 1], 1)
}
[pscustomobject]@{
    Display = $DisplayNumber
    Samples = $samples.Count
    MedianMs = Get-Percentile 0.5
    P95Ms = Get-Percentile 0.95
    ObservationsMs = $samples
}
