using System.ComponentModel;
using System.Diagnostics;
using Microsoft.Win32;

namespace SmartTaskbar;

internal sealed class IslandWidthObserver : IDisposable
{
    private IntPtr _preferred;
    private int _pending;
    private int _sent;
    private int _writing;
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
        if (flyoutBar != IntPtr.Zero)
            _preferred = flyoutBar;
        var bar = bars.Find(item => item.Handle == _preferred);
        if (bar.Handle == IntPtr.Zero)
            bar = bars.Find(item => item.Handle == primary);
        if (bar.Handle == IntPtr.Zero || bar.Edge is TaskbarPosition.Left or TaskbarPosition.Right ||
            visuals.OpenIslandBounds(bar.Handle) is not { } island)
        {
            _pending = 0;
            _sent = 0;
            return;
        }
        var width = IslandMenuWidth.Measure(island.Width, bar.Display.Width, Fun.GetDpiForWindow(bar.Handle));
        if (width != _pending)
        {
            _pending = width;
            _settledAt = now + 300;
            return;
        }
        if (now < _settledAt || now < _retryAfter || width == _sent || _writer != null)
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
            _writer = Process.Start(info);
            _writing = width;
            if (_writer != null)
                _retryAfter = 0;
        }
        catch (Exception error) when (error is IOException or Win32Exception or UnauthorizedAccessException)
        {
            Debug.WriteLine(error.Message);
        }
    }

    public void Dispose() => _writer?.Dispose();
}
