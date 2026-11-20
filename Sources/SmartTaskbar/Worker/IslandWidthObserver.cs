using System.ComponentModel;
using System.Diagnostics;
using Microsoft.Win32;

namespace SmartTaskbar;

internal sealed class IslandWidthObserver : IDisposable
{
    private IntPtr _preferred;
    private readonly record struct Geometry(int Width, int? TrayShift, int SearchInset);
    private Geometry? _pending;
    private Geometry? _sent;
    private Geometry _writing;
    private readonly EventWaitHandle _openEvent = new(false, EventResetMode.ManualReset, IslandMenuWidth.FlyoutOpenEvent);
    private bool _menuOpen;
    private long _settledAt;
    private long _retryAfter;
    private Process? _writer;

    internal void Observe(List<TaskbarInfo> bars, IntPtr primary, IntPtr flyoutBar, TaskbarVisuals visuals, long now)
    {
        if (_writer is { HasExited: true } completed)
        {
            if (completed.ExitCode == 0)
                _sent = _writing;
            else
                _retryAfter = now + 5_000;
            completed.Dispose();
            _writer = null;
        }
        var open = flyoutBar != IntPtr.Zero;
        if (_menuOpen != open)
        {
            _menuOpen = open;
            if (open) _openEvent.Set(); else _openEvent.Reset();
        }
        if (open)
        {
            _preferred = flyoutBar;
            return;
        }
        var primaryBar = bars.Find(item => item.Handle == primary);
        if (primaryBar.Handle == IntPtr.Zero)
            return;
        var bar = bars.Find(item => item.Handle == _preferred);
        if (bar.Handle == IntPtr.Zero)
            bar = primaryBar;
        int? trayShift = null;
        var searchInset = 0;
        if (primaryBar.Edge == TaskbarPosition.Bottom)
        {
            if (visuals.OpenIslandBounds(primary) is not { } primaryIsland)
            {
                _pending = _sent = null;
                return;
            }
            var monitor = MonitorInfo.Create();
            var work = Fun.GetMonitorInfo(primaryBar.Monitor, ref monitor) ? monitor.WorkArea.ToRectangle() : primaryBar.Display;
            trayShift = IslandMenuWidth.TrayShift(primaryIsland, work, Fun.GetDpiForWindow(primary));
            if (bar.Handle == primary)
                searchInset = IslandMenuWidth.SearchInset(bar.ShownBounds, work, Fun.GetDpiForWindow(primary));
        }
        if (bar.Handle != primary && bar.Edge == TaskbarPosition.Bottom)
        {
            var monitor = MonitorInfo.Create();
            if (Fun.GetMonitorInfo(bar.Monitor, ref monitor))
                searchInset = IslandMenuWidth.SearchInset(bar.ShownBounds, monitor.WorkArea.ToRectangle(), Fun.GetDpiForWindow(bar.Handle));
        }
        var width = bar.Edge is TaskbarPosition.Top or TaskbarPosition.Bottom &&
            visuals.OpenIslandBounds(bar.Handle) is { } island
            ? IslandMenuWidth.Measure(island.Width, bar.Display.Width, Fun.GetDpiForWindow(bar.Handle)) : 0;
        var geometry = new Geometry(width, trayShift, searchInset);
        if (geometry != _pending)
        {
            _pending = geometry;
            _settledAt = now + 300;
            return;
        }
        if (now < _settledAt || now < _retryAfter || geometry == _sent || _writer != null)
            return;
        _retryAfter = now + 5_000;
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(IslandMenuWidth.Preferences);
            if (key?.GetValue(IslandMenuWidth.CompanionSetting) is not string client ||
                !Path.IsPathFullyQualified(client) || !string.Equals(Path.GetFileName(client),
                    "SmartTaskbar.Appearance.exe", StringComparison.OrdinalIgnoreCase) || !File.Exists(client))
                return;
            var info = new ProcessStartInfo(client) { UseShellExecute = false, CreateNoWindow = true };
            info.ArgumentList.Add(IslandMenuWidth.Argument);
            info.ArgumentList.Add(width.ToString(System.Globalization.CultureInfo.InvariantCulture));
            info.ArgumentList.Add(trayShift?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "native");
            info.ArgumentList.Add(searchInset.ToString(System.Globalization.CultureInfo.InvariantCulture));
            _writer = Process.Start(info);
            _writing = geometry;
            if (_writer != null)
                _retryAfter = 0;
        }
        catch (Exception error) when (error is IOException or Win32Exception or UnauthorizedAccessException)
        {
            Debug.WriteLine(error.Message);
        }
    }

    public void Reset()
    {
        _openEvent.Reset();
        _menuOpen = false;
        _preferred = IntPtr.Zero;
        _pending = _sent = null;
        _retryAfter = 0;
    }

    public void Dispose()
    {
        Reset();
        _openEvent.Dispose();
        _writer?.Dispose();
    }
}
