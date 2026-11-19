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
        private const int CloseWindow = 0x0010;
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
        private TaskbarInfo? _seamReveal;
        private bool _seamConfirmed;
        private IntPtr _dismissBar;
        private long _dismissAfter;

        public bool MenuOpen { get; set; }
        public event EventHandler? CloseRequested;

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
            if (message.Msg == CloseWindow && CloseRequested is { } close)
            {
                close(this, EventArgs.Empty);
                return;
            }

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
            if (!_cursorSampler.Enabled || (_seamReveal is { } reveal && !_bars.Contains(reveal)))
                ClearSeam();

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

            if (_mode != AutoModeType.Auto || _bars.Count == 0 || _seamReveal.HasValue || MenuOpen)
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
            if (_seamReveal is { } active)
            {
                if (active.ShownBounds.Contains(cursor))
                {
                    if (!_seamConfirmed)
                    {
                        _seamConfirmed = TaskbarHelper.IsShown(active);
                        if (!_seamConfirmed && _dwell.TryRequestReveal(now))
                            TaskbarHelper.Reveal(_shell, active.Monitor);
                    }

                    return;
                }

                if (_mode == AutoModeType.Dock)
                {
                    _dismissBar = active.Handle;
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

                if (!_dwell.IsReady(bar.Handle, now))
                    return;

                var shown = TaskbarHelper.IsShown(bar);
                if (shown || (_dwell.TryRequestReveal(now) && TaskbarHelper.Reveal(_shell, bar.Monitor)))
                {
                    _seamReveal = bar;
                    _seamConfirmed = shown;
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
            _seamReveal = null;
            _seamConfirmed = false;
        }

        public async Task WaitForNativeHideOnExitAsync()
        {
            SetMode(AutoModeType.None);
            if (!IsAutoHide())
                return;

            var shell = FindWindow(TaskbarHelper.PrimaryClass, null);
            var displays = Array.ConvertAll(Screen.AllScreens, screen => screen.Bounds);
            TaskbarHelper.ReadTaskbars(shell, displays, _bars);

            // Toggling auto-hide off while a tray is still open can leave it stuck after the next toggle.
            var deadline = Environment.TickCount64 + 2_200;
            var hideRequests = 0;
            var nextHide = Environment.TickCount64 + SeamDismissMs;
            while (Environment.TickCount64 < deadline && _bars.Exists(TaskbarHelper.IsBeyondAutoHideStrip))
            {
                var cursor = Cursor.Position;
                var now = Environment.TickCount64;
                if (hideRequests < 2 && now >= nextHide &&
                    !_bars.Exists(bar => bar.ShownBounds.Contains(cursor)) &&
                    !TaskbarHelper.IsTraySurface(WindowAt(cursor).GetClassName()))
                {
                    TaskbarHelper.Hide(shell);
                    hideRequests++;
                    nextHide = now + CommandCooldownMs;
                }

                await Task.Delay(90);
            }
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
