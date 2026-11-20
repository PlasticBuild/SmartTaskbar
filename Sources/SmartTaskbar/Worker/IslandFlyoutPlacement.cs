using System.Diagnostics;
using Microsoft.Win32;
using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar
{
    using static Fun;

    internal sealed class IslandFlyoutPlacement : IDisposable
    {
        private const uint MoveOnly = 0x0001 | 0x0004 | 0x0010 | 0x0200 | 0x4000;
        private const int Duration = 185;
        private readonly Timer _slide = new() { Interval = 15 };
        private readonly IslandMenuJoin _join = new();
        private readonly MenuExitAnimation _exit = new();
        private IntPtr _window;
        private int _process;
        private Rectangle _original;
        private Rectangle _requested;
        private Rectangle _target;
        private long _placementDeadline;
        private long _started;
        private int _travel;
        private int _radius;
        private bool _animate;
        private IslandMenuSurface? _kind;

        public IslandFlyoutPlacement() => _slide.Tick += (_, _) => SlideFrame();

        public void Apply(IntPtr window, TaskbarInfo bar, Rectangle? island, long now)
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
                if (GetWindowThreadProcessId(window, out _process) == 0 || !GetWindowRect(window, out var raw))
                    return;
                _kind = Identify(window, _process, raw.ToRectangle(), bar.Display, surface);
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
            var x = actual.X;
            if (kind is IslandMenuSurface.QuickSettings or IslandMenuSurface.Notifications)
                x = Math.Clamp(surface.Right + Scale(12, dpi) - actual.Width, bar.Display.Left, bar.Display.Right - actual.Width);
            else if (kind == IslandMenuSurface.Search)
            {
                var face = IslandMenuJoin.Face(kind, actual, surface, dpi);
                var center = surface.Left + surface.Width / 2;
                var left = Math.Clamp(center - face.Width / 2, bar.Display.Left, bar.Display.Right - face.Width);
                x += left - face.Left;
            }
            var desired = new Rectangle(x, surface.Top + Scale(18, dpi) - actual.Height, actual.Width, actual.Height);
            _join.Apply(window, bar, surface, desired, kind, _animate ? Duration : 0, _radius);
            if (_animate && !SystemInformation.HighContrast)
            {
                var face = IslandMenuJoin.Face(kind, desired, surface, dpi);
                face = Rectangle.FromLTRB(face.Left, Math.Max(face.Top, bar.Display.Top), face.Right, surface.Top + Scale(3, dpi));
                _exit.Observe(window, desired, face, dpi, _radius, _join.MaterialColor);
            }

            var firstPlacement = _target.IsEmpty;
            if (desired != _target)
                _placementDeadline = now + 1_500;
            _target = desired;
            if (firstPlacement)
            {
                _original = actual;
                _started = now;
                _travel = _animate ? Scale(14, dpi) : 0;
                if (_travel > 0)
                {
                    SlideFrame();
                    _slide.Start();
                    return;
                }
            }
            if (!_slide.Enabled && now <= _placementDeadline && desired != actual)
                Move(desired);
        }

        private void SlideFrame()
        {
            if (_target.IsEmpty || !IsWindowVisible(_window) ||
                GetWindowThreadProcessId(_window, out var process) == 0 || process != _process)
            {
                _slide.Stop();
                return;
            }
            var elapsed = Math.Clamp((Environment.TickCount64 - _started) / (double)Duration, 0, 1);
            var frame = _target;
            frame.Offset(0, (int)Math.Round(_travel * Math.Pow(1 - elapsed, 2)));
            if (_requested != frame || elapsed == 1)
                Move(frame);
            if (elapsed == 1) _slide.Stop();
        }

        private void Move(Rectangle frame)
        {
            if (SetWindowPos(_window, IntPtr.Zero, frame.X, frame.Y, 0, 0, MoveOnly))
                _requested = frame;
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
            _slide.Stop();
            _join.Close(animateClose && _animate);
            _exit.Close(animateClose && _animate);
            if (!_requested.IsEmpty && GetWindowThreadProcessId(_window, out var process) != 0 && process == _process &&
                GetWindowRect(_window, out var rect) && rect.ToRectangle() == _requested && _original != _requested)
                SetWindowPos(_window, IntPtr.Zero, _original.X, _original.Y, 0, 0, MoveOnly);
            _window = IntPtr.Zero;
            _requested = _target = Rectangle.Empty;
            _kind = null;
        }

        public void Dispose()
        {
            Reset();
            _slide.Dispose();
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
