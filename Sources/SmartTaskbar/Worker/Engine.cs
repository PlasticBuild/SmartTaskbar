using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar
{
    using static Fun;

    internal sealed class Engine : NativeWindow, IDisposable
    {
        private const uint ForegroundChanged = 0x0003;
        private const uint MoveSizeEnded = 0x000B;
        private const uint MinimizeEnded = 0x0017;
        private const uint LocationChanged = 0x800B;
        private const uint SkipOwnProcess = 2;
        private const int SettingChanged = 0x001A;
        private const int DisplayChanged = 0x007E;
        private const int SeamDismissMs = 240;
        private const int CommandCooldownMs = 550;

        private readonly List<TaskbarInfo> _bars = new();
        private readonly List<(TaskbarInfo Bar, Rectangle Band)> _seams = new();
        private readonly Timer _update = new() { Interval = 65 };
        private readonly Timer _watchdog = new() { Interval = 1250 };
        private readonly Timer _cursorSampler = new() { Interval = 80 };
        private readonly WinEventCallback _onWindowEvent;
        private readonly IntPtr[] _hooks = new IntPtr[3];
        private readonly uint _taskbarCreated = RegisterWindowMessage("TaskbarCreated");

        private IntPtr _shell;
        private IntPtr _foreground;
        private IntPtr _locationHook;
        private AutoModeType _mode;
        private bool _topologyDirty = true;
        private long _nextRefresh;
        private long _nextCommand;
        private SharedEdgeDwell _dwell;
        private IntPtr _seamReveal;
        private Rectangle _seamRevealBounds;
        private IntPtr _dismissBar;
        private long _dismissAfter;

        public bool MenuOpen { get; set; }

        public Engine()
        {
            _onWindowEvent = OnWindowEvent;
            _update.Tick += (_, _) =>
            {
                _update.Stop();
                Update();
            };
            _watchdog.Tick += (_, _) => Update();
            _cursorSampler.Tick += (_, _) => SampleSharedEdge();

            // A top-level hidden window receives Explorer and display broadcasts.
            CreateHandle(new CreateParams { Caption = "SmartTaskbar Monitor" });
        }

        public void SetMode(AutoModeType mode)
        {
            if (_mode == mode)
                return;

            if (_mode == AutoModeType.Auto)
            {
                foreach (var hook in _hooks)
                    if (hook != IntPtr.Zero)
                        UnhookWinEvent(hook);
                WatchForegroundLocation(IntPtr.Zero);
            }

            _mode = mode;
            if (mode == AutoModeType.None)
            {
                _update.Stop();
                _watchdog.Stop();
                _cursorSampler.Stop();
                ClearSeam();
                _dismissBar = IntPtr.Zero;
                _nextCommand = 0;
                _shell = IntPtr.Zero;
                return;
            }

            if (mode == AutoModeType.Auto)
            {
                _dismissBar = IntPtr.Zero;
                _hooks[0] = WatchEvent(ForegroundChanged);
                _hooks[1] = WatchEvent(MoveSizeEnded);
                _hooks[2] = WatchEvent(MinimizeEnded);
            }

            _topologyDirty = true;
            _watchdog.Interval = mode == AutoModeType.Dock ? 2500 : 1250;
            _watchdog.Start();
            QueueUpdate();
        }

        private IntPtr WatchEvent(uint eventType, uint thread = 0)
            => SetWinEventHook(eventType, eventType, IntPtr.Zero, _onWindowEvent, 0, thread, SkipOwnProcess);

        private void WatchForegroundLocation(IntPtr hwnd)
        {
            if (_foreground == hwnd)
                return;

            if (_locationHook != IntPtr.Zero)
                UnhookWinEvent(_locationHook);

            _foreground = hwnd;
            var thread = hwnd == IntPtr.Zero ? 0 : GetWindowThreadProcessId(hwnd, out _);
            _locationHook = thread == 0 ? IntPtr.Zero : WatchEvent(LocationChanged, (uint)thread);
        }

        private void OnWindowEvent(IntPtr hook, uint eventType, IntPtr hwnd,
            int idObject, int idChild, uint threadId, uint time)
        {
            if (eventType != LocationChanged || (hwnd == _foreground && idObject == 0 && idChild == 0))
                QueueUpdate();
        }

        protected override void WndProc(ref Message message)
        {
            if (message.Msg == _taskbarCreated || message.Msg is DisplayChanged or SettingChanged)
            {
                _topologyDirty = true;
                QueueUpdate();
            }

            base.WndProc(ref message);
        }

        private void QueueUpdate()
        {
            if (_mode != AutoModeType.None && !_update.Enabled)
                _update.Start();
        }

        private void RefreshTaskbars(long now)
        {
            var shell = FindWindow(TaskbarHelper.PrimaryClass, null);
            if (_shell != shell)
            {
                _shell = shell;
                ClearSeam();
                _dismissBar = IntPtr.Zero;
                _nextCommand = 0;
            }

            if (_shell != IntPtr.Zero)
                SetAutoHide(true);
            var displays = Array.ConvertAll(Screen.AllScreens, screen => screen.Bounds);
            TaskbarHelper.ReadTaskbars(_shell, displays, _bars);
            _seams.Clear();
            foreach (var bar in _bars)
            {
                foreach (var display in displays)
                {
                    var band = bar.SharedEdgeBand(display);
                    if (band.Width > 0 && band.Height > 0)
                        _seams.Add((bar, band));
                }
            }

            _cursorSampler.Enabled = _seams.Count > 0;
            if (!_cursorSampler.Enabled || !_bars.Exists(bar => bar.Handle == _seamReveal
                && bar.ShownBounds == _seamRevealBounds))
            {
                _seamReveal = IntPtr.Zero;
            }

            _topologyDirty = false;
            _nextRefresh = now + 2_500;
        }

        private void Update()
        {
            if (_mode == AutoModeType.None)
                return;

            var now = Environment.TickCount64;
            if (_topologyDirty || now >= _nextRefresh)
                RefreshTaskbars(now);

            if (_mode != AutoModeType.Auto || _bars.Count == 0 || _seamReveal != IntPtr.Zero || MenuOpen)
                return;

            var foreground = GetForegroundWindow();
            WatchForegroundLocation(foreground);
            var foregroundClass = foreground.GetClassName();
            if (TaskbarHelper.IsTraySurface(foregroundClass))
                return;

            var desktop = TaskbarHelper.IsDesktop(foregroundClass);
            var window = Rectangle.Empty;
            if (!desktop)
            {
                if (foreground == IntPtr.Zero || foreground.IsWindowInvisible()
                    || !TryGetVisibleBounds(foreground, out var bounds))
                    return;
                window = bounds.ToRectangle();
            }

            var cursor = Cursor.Position;
            var cursorMonitor = MonitorFromPoint(new TagPoint { x = cursor.X, y = cursor.Y }, 0);
            var activeMonitor = desktop ? cursorMonitor : MonitorFromWindow(foreground, 2);
            var maximized = !desktop && IsZoomed(foreground);
            var hovered = false;
            var coveredBarShown = false;
            TaskbarInfo? preferred = null;
            var preferredShown = false;
            var bestPriority = 0;

            foreach (var bar in _bars)
            {
                var shown = TaskbarHelper.IsShown(bar);
                if (shown && bar.ShownBounds.Contains(cursor))
                    hovered |= IsTaskbarInteraction(bar, cursor);

                if (bar.IsCoveredBy(window, activeMonitor, maximized))
                {
                    coveredBarShown |= shown;
                    continue;
                }

                if (!desktop && bar.Monitor != activeMonitor && !shown && !IsDesktopExposed(bar))
                    continue;

                var priority = bar.Monitor == activeMonitor ? 3 : bar.Monitor == cursorMonitor ? 2 : 1;
                if (priority <= bestPriority)
                    continue;

                preferred = bar;
                preferredShown = shown;
                bestPriority = priority;
            }

            if (hovered || now < _nextCommand)
                return;

            // Explorer normally keeps one programmatically revealed tray active at a time.
            if (preferred is { } target && !preferredShown)
            {
                if (TaskbarHelper.Reveal(_shell, target.Monitor))
                    _nextCommand = now + CommandCooldownMs;
            }
            else if (coveredBarShown)
            {
                TaskbarHelper.Hide(_shell);
                _nextCommand = now + CommandCooldownMs;
            }
        }

        private static bool IsTaskbarInteraction(TaskbarInfo bar, Point cursor)
        {
            var hit = WindowAt(cursor);
            return hit == bar.Handle || TaskbarHelper.IsTraySurface(hit.GetClassName());
        }

        private static IntPtr WindowAt(Point point)
            => GetAncestor(WindowFromPoint(new TagPoint { x = point.X, y = point.Y }), TaskbarHelper.RootWindow);

        private static bool IsDesktopExposed(TaskbarInfo bar)
        {
            var bounds = bar.ShownBounds;
            var horizontal = bar.Edge is TaskbarPosition.Top or TaskbarPosition.Bottom;
            for (var part = 1; part <= 3; part++)
            {
                var point = new TagPoint
                {
                    x = bounds.Left + bounds.Width * (horizontal ? part : 2) / 4,
                    y = bounds.Top + bounds.Height * (horizontal ? 2 : part) / 4
                };
                var hit = GetAncestor(WindowFromPoint(point), TaskbarHelper.RootWindow);
                if (!TaskbarHelper.IsDesktop(hit.GetClassName()))
                    return false;
            }

            return true;
        }

        private void SampleSharedEdge()
        {
            var cursor = Cursor.Position;
            var now = Environment.TickCount64;
            if (_seamReveal != IntPtr.Zero)
            {
                if (_seamRevealBounds.Contains(cursor))
                    return;

                if (_mode == AutoModeType.Dock)
                {
                    _dismissBar = _seamReveal;
                    _dismissAfter = now + SeamDismissMs;
                }

                ClearSeam();
                if (_mode == AutoModeType.Auto)
                    QueueUpdate();
            }

            foreach (var (bar, band) in _seams)
            {
                if (!band.Contains(cursor))
                    continue;

                if (_dwell.IsReady(bar.Handle, now) &&
                    (TaskbarHelper.IsShown(bar) || TaskbarHelper.Reveal(_shell, bar.Monitor)))
                {
                    _seamReveal = bar.Handle;
                    _seamRevealBounds = bar.ShownBounds;
                    _dismissBar = IntPtr.Zero;
                    _nextCommand = now + CommandCooldownMs;
                }

                return;
            }

            _dwell = default;
            DismissSeamReveal(cursor, now);
        }

        private void DismissSeamReveal(Point cursor, long now)
        {
            if (_dismissBar == IntPtr.Zero || now < _dismissAfter || MenuOpen)
                return;

            var hitClass = WindowAt(cursor).GetClassName();
            if (TaskbarHelper.IsTraySurface(hitClass) || TaskbarHelper.IsTraySurface(GetForegroundWindow().GetClassName()))
            {
                _dismissAfter = now + SeamDismissMs;
                return;
            }

            // A native reveal on a different display takes precedence over our pending dismiss.
            var shownCount = 0;
            var requestedBarShown = false;
            foreach (var bar in _bars)
            {
                if (!TaskbarHelper.IsShown(bar))
                    continue;
                shownCount++;
                requestedBarShown |= bar.Handle == _dismissBar;
            }
            if (requestedBarShown && shownCount == 1)
                TaskbarHelper.Hide(_shell);

            _dismissBar = IntPtr.Zero;
        }

        private void ClearSeam()
        {
            _dwell = default;
            _seamReveal = IntPtr.Zero;
        }

        public void Dispose()
        {
            SetMode(AutoModeType.None);
            _update.Dispose();
            _watchdog.Dispose();
            _cursorSampler.Dispose();
            DestroyHandle();
        }
    }
}
