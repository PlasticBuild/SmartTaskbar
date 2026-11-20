using System.Diagnostics;
using Microsoft.Win32;

namespace SmartTaskbar
{
    using static Fun;

    internal sealed class IslandFlyoutPlacement : IDisposable
    {
        private const int Duration = 185;
        private readonly IslandMenuJoin _join = new();
        private readonly MenuExitAnimation _exit = new();
        private IntPtr _window;
        private int _radius;
        private bool _animate;
        private IslandMenuSurface? _kind;
        public bool IsSearchOpen => _kind == IslandMenuSurface.Search;

        public void Apply(IntPtr window, TaskbarInfo bar, Rectangle? island)
        {
            if (window == IntPtr.Zero || bar.Edge != TaskbarPosition.Bottom || island is not { } surface)
            {
                End(animateClose: window == IntPtr.Zero);
                return;
            }

            var fresh = window != _window;
            if (fresh)
            {
                End(animateClose: false);
                _exit.Close(false);
                _window = window;
                if (GetWindowThreadProcessId(window, out var process) == 0 || !GetWindowRect(window, out var raw))
                    return;
                _kind = Identify(window, process, raw.ToRectangle(), bar.Display, surface);
                if (_kind.HasValue)
                {
                    _animate = MenuAnimationSettings.Read();
                    using var settings = Registry.CurrentUser.OpenSubKey(@"Software\SmartTaskbar\Desktop");
                    _radius = Math.Clamp(settings?.GetValue("IslandCornerRadius") as int? ?? 4, 0, 24);
                }
            }
            if (_kind is not { } kind || !GetWindowRect(window, out var rect))
                return;
            var actual = rect.ToRectangle();
            if (actual.IsEmpty || actual.Right <= bar.Display.Left || actual.Left >= bar.Display.Right ||
                actual.Width > bar.Display.Width || Math.Abs(actual.Bottom - surface.Top) > bar.ShownBounds.Height * 2)
                return;

            var dpi = (int)GetDpiForWindow(bar.Handle);
            if (dpi == 0) dpi = 96;
            // Explorer positions the shell surface before presenting it. Animate the
            // connection on our own layer so native controls stay at their opening coordinates.
            _join.Apply(window, bar, surface, actual, kind, _animate ? Duration : 0, _radius);
            if (_animate && !SystemInformation.HighContrast)
            {
                var face = IslandMenuJoin.Face(kind, actual, surface, bar.Display, dpi);
                face = Rectangle.FromLTRB(face.Left, Math.Max(face.Top, bar.Display.Top), face.Right, surface.Top + Scale(3, dpi));
                _exit.Observe(window, actual, face, dpi, _radius, _join.MaterialColor);
            }
        }

        public void Reset()
        {
            End(animateClose: false);
            _join.Close(false);
            _exit.Close(false);
        }

        private void End(bool animateClose)
        {
            if (_window == IntPtr.Zero)
                return;
            _join.Close(animateClose && _animate);
            _exit.Close(animateClose && _animate);
            _window = IntPtr.Zero;
            _kind = null;
        }

        public void Dispose()
        {
            Reset();
            _join.Dispose();
            _exit.Dispose();
        }

        private static int Scale(int dips, int dpi) => (dips * dpi + 48) / 96;

        private static IslandMenuSurface? Identify(IntPtr window, int processId, Rectangle bounds, Rectangle display, Rectangle island)
        {
            var name = window.GetClassName();
            if (name is not ("ControlCenterWindow" or "NotificationCenterWindow" or "Windows.UI.Core.CoreWindow" or
                "TopLevelWindowForOverflowXamlIsland" or "NotifyIconOverflowWindow"))
                return null;
            try
            {
                using var process = Process.GetProcessById(processId);
                return process.ProcessName switch
                {
                    "StartMenuExperienceHost" => IslandMenuSurface.Start,
                    "SearchHost" => IslandMenuSurface.Search,
                    "ShellHost" when name == "ControlCenterWindow" => IslandMenuSurface.QuickSettings,
                    "ShellExperienceHost" when name == "NotificationCenterWindow" ||
                        (bounds.Top <= display.Top + 24 && bounds.Height >= display.Height * .7) => IslandMenuSurface.Notifications,
                    "explorer" when name is "TopLevelWindowForOverflowXamlIsland" or "NotifyIconOverflowWindow" => IslandMenuSurface.Overflow,
                    "ShellExperienceHost" when name == "Windows.UI.Core.CoreWindow" &&
                        Math.Abs(bounds.Bottom - island.Top) <= island.Height && GetCursorPos(out var cursor) &&
                        island.Contains(cursor.x, cursor.y) => IslandMenuSurface.JumpList,
                    _ => null
                };
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or System.ComponentModel.Win32Exception)
            {
                return null;
            }
        }
    }
}
