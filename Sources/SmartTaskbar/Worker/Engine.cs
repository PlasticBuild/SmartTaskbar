using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar
{
    using static Fun;

    internal sealed class Engine : NativeWindow, IDisposable
    {
        private const uint ForegroundChanged = 0x0003;
        private const uint MoveSizeEnded = 0x000B;
        private const uint MinimizeEnded = 0x0017;
        private const uint ObjectDestroyed = 0x8001;
        private const uint ObjectHidden = 0x8003;
        private const uint LocationChanged = 0x800B;
        private const uint SkipOwnProcess = 2;
        private const int CloseWindow = 0x0010;
        private const int SettingChanged = 0x001A;
        private const int DisplayChanged = 0x007E;
        private const int Hotkey = 0x0312;
        private const int RestoreHotkeyId = 1;
        private const int ConvertibleSlateMode = 0x2003;
        private const uint RestoreModifiers = 0x4000 | 0x0004 | 0x0002 | 0x0001; // No repeat, Shift, Ctrl, Alt
        private const string WindowCaption = "SmartTaskbar Monitor";
        private const int EdgeDismissMs = 240;
        private const int ExitHideRetryMs = 550;
        private static readonly uint ShowTaskbarsMessage =
            RegisterWindowMessage("SmartTaskbar.ShowTaskbars.959d3545-aa5c-42a8-a327-6e2c079daa94");

        private readonly List<TaskbarInfo> _bars = new();
        private readonly List<(TaskbarInfo Bar, Rectangle Band)> _edges = new();
        private readonly Timer _update = new() { Interval = 65 };
        private readonly Timer _watchdog = new() { Interval = 1250 };
        private readonly Timer _cursorSampler = new() { Interval = 80 };
        private readonly TaskbarVisuals _visuals = new();
        private readonly Func<bool> _isTablet;
        private readonly WinEventCallback _onWindowEvent;
        private readonly EnumWindowCallback _inspectWindow;
        private readonly IntPtr[] _hooks = new IntPtr[4];
        private readonly uint _taskbarCreated = RegisterWindowMessage("TaskbarCreated");

        private IntPtr _shell;
        private IntPtr _foreground;
        private IntPtr _locationHook;
        private AutoModeType _mode;
        private bool _topologyDirty = true;
        private bool _autoHideEstablished;
        private bool _nativeTaskbarReady;
        private bool _tablet;
        private TaskbarCoverage[] _coverage = [];
        private long _nextRefresh;
        private EdgeDwell _dwell;
        private TaskbarInfo? _edgeReveal;
        private bool _edgeConfirmed;
        private IntPtr _visualPeek;
        private IntPtr _dismissBar;
        private long _dismissAfter;

        public bool MenuOpen { get; set; }
        public bool RestoreHotkeyRegistered { get; private set; }
        public event EventHandler? CloseRequested;
        public event EventHandler? ShowTaskbarsRequested;
        public event EventHandler? NativeAutoHideEnabled;

        public Engine(Func<bool>? isTablet = null)
        {
            _isTablet = isTablet ?? (() => GetSystemMetrics(ConvertibleSlateMode) == 0);
            _onWindowEvent = OnWindowEvent;
            _inspectWindow = InspectWindow;
            _update.Tick += (_, _) =>
            {
                _update.Stop();
                Update();
            };
            _watchdog.Tick += (_, _) => Update();
            _cursorSampler.Tick += (_, _) =>
            {
                if (_mode == AutoModeType.Auto)
                    SampleVisualEdges();
                else
                    SampleTaskbarEdges();
            };

            // A top-level hidden window receives Explorer and display broadcasts.
            CreateHandle(new CreateParams { Caption = WindowCaption });
        }

        internal static bool TryRequestShowTaskbars()
        {
            var window = FindWindow(null, WindowCaption);
            return window != IntPtr.Zero && ShowTaskbarsMessage != 0 &&
                PostMessage(window, ShowTaskbarsMessage, IntPtr.Zero, IntPtr.Zero);
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

            _visuals.Reset();
            ClearEdgeReveal();
            _visualPeek = IntPtr.Zero;
            _mode = mode;
            _autoHideEstablished = false;
            _nativeTaskbarReady = false;
            if (mode == AutoModeType.None)
            {
                _update.Stop();
                _watchdog.Stop();
                _cursorSampler.Stop();
                ClearEdgeReveal();
                _dismissBar = IntPtr.Zero;
                _shell = IntPtr.Zero;
                if (RestoreHotkeyRegistered)
                {
                    UnregisterHotKey(Handle, RestoreHotkeyId);
                    RestoreHotkeyRegistered = false;
                }
                return;
            }

            if (!RestoreHotkeyRegistered)
                RestoreHotkeyRegistered = RegisterHotKey(Handle, RestoreHotkeyId, RestoreModifiers, (uint)Keys.T);

            if (mode == AutoModeType.Auto)
            {
                _dismissBar = IntPtr.Zero;
                _hooks[0] = WatchEvent(ForegroundChanged);
                _hooks[1] = WatchEvent(MoveSizeEnded);
                _hooks[2] = WatchEvent(MinimizeEnded);
                _hooks[3] = SetWinEventHook(ObjectDestroyed, ObjectHidden, IntPtr.Zero, _onWindowEvent,
                    0, 0, SkipOwnProcess);
            }

            _topologyDirty = true;
            _watchdog.Interval = mode == AutoModeType.Dock ? 2500 : 850;
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
            if (eventType >= ObjectDestroyed && (hwnd == IntPtr.Zero || idObject != 0 || idChild != 0))
                return;
            if (eventType == LocationChanged && hwnd != _foreground)
                return;

            QueueUpdate();
        }

        protected override void WndProc(ref Message message)
        {
            if ((message.Msg == Hotkey && message.WParam == (IntPtr)RestoreHotkeyId) ||
                (ShowTaskbarsMessage != 0 && message.Msg == ShowTaskbarsMessage))
            {
                ShowTaskbarsRequested?.Invoke(this, EventArgs.Empty);
                return;
            }

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

        private void QueueUpdate(int delay = 65)
        {
            if (_mode == AutoModeType.None || (_update.Enabled && _update.Interval <= delay))
                return;

            _update.Stop();
            _update.Interval = delay;
            _update.Start();
        }

        private void RefreshTaskbars(long now)
        {
            var shell = FindWindow(TaskbarHelper.PrimaryClass, null);
            if (_shell != shell)
            {
                _shell = shell;
                ClearEdgeReveal();
                _dismissBar = IntPtr.Zero;
                _autoHideEstablished = false;
                _nativeTaskbarReady = false;
                _visualPeek = IntPtr.Zero;
            }

            var displays = Array.ConvertAll(Screen.AllScreens, screen => screen.Bounds);
            TaskbarHelper.ReadTaskbars(_shell, displays, _bars);
            _visuals.SetLayout(_bars);
            if (_coverage.Length != _bars.Count)
                _coverage = new TaskbarCoverage[_bars.Count];
            _edges.Clear();
            foreach (var bar in _bars)
            {
                var band = bar.RevealBand();
                if (band.Width > 0 && band.Height > 0)
                    _edges.Add((bar, band));
            }

            if (_edgeReveal is { } reveal && !_bars.Contains(reveal))
                ClearEdgeReveal();

            _topologyDirty = false;
            _nextRefresh = now + 2_500;
        }

        private void Update()
        {
            if (_mode == AutoModeType.None)
                return;

            if (_isTablet())
            {
                if (!_tablet)
                {
                    _tablet = true;
                    _visuals.Reset();
                    _cursorSampler.Stop();
                    ClearEdgeReveal();
                    _visualPeek = IntPtr.Zero;
                    _dismissBar = IntPtr.Zero;
                    if (_mode == AutoModeType.Dock && _autoHideEstablished)
                        SetAutoHide(false);
                    _autoHideEstablished = false;
                    _nativeTaskbarReady = false;
                }
                return;
            }

            if (_tablet)
            {
                _tablet = false;
                _topologyDirty = true;
            }

            var now = Environment.TickCount64;
            if (_topologyDirty || now >= _nextRefresh)
                RefreshTaskbars(now);

            if (_shell == IntPtr.Zero || _bars.Count == 0)
                return;

            var nativeAutoHide = IsAutoHide();
            if (_mode == AutoModeType.Dock)
            {
                if (_autoHideEstablished && !nativeAutoHide)
                {
                    ShowTaskbarsRequested?.Invoke(this, EventArgs.Empty);
                    return;
                }

                if (!nativeAutoHide)
                    SetAutoHide(true);
                _autoHideEstablished = nativeAutoHide || IsAutoHide();
                _cursorSampler.Enabled = _autoHideEstablished && _edges.Count > 0;
                return;
            }

            if (nativeAutoHide)
            {
                if (_nativeTaskbarReady)
                    NativeAutoHideEnabled?.Invoke(this, EventArgs.Empty);
                else
                    SetAutoHide(false);
                return;
            }
            _nativeTaskbarReady = true;

            var foreground = GetForegroundWindow();
            WatchForegroundLocation(foreground);
            var shellInUse = MenuOpen || TaskbarHelper.IsTraySurface(foreground.GetClassName());
            Array.Clear(_coverage);
            EnumWindows(_inspectWindow, IntPtr.Zero);

            var covered = Array.Exists(_coverage, value => value != TaskbarCoverage.Clear);
            if (!covered)
            {
                _visualPeek = IntPtr.Zero;
                _dwell = default;
            }
            _cursorSampler.Enabled = covered && !shellInUse;

            var reveal = _visualPeek;
            var obscuredPeek = IntPtr.Zero;
            if (GetCursorPos(out var point))
            {
                var mouse = new Point(point.x, point.y);
                if (_visualPeek != IntPtr.Zero &&
                    !_bars.Exists(bar => bar.Handle == _visualPeek && bar.ShownBounds.Contains(mouse)))
                {
                    _visualPeek = IntPtr.Zero;
                    reveal = IntPtr.Zero;
                }

                if (shellInUse)
                {
                    var monitor = MonitorFromWindow(foreground, 0);
                    if (MenuOpen || monitor == IntPtr.Zero || _bars.Exists(bar => bar.ShownBounds.Contains(mouse)))
                        monitor = MonitorFromPoint(point, 0);

                    // A tray or Start menu interaction should not reveal a full display's other bar.
                    reveal = _bars.Find(bar => bar.Monitor == monitor).Handle;
                }
                else if (covered && _visualPeek == IntPtr.Zero)
                {
                    for (var index = 0; index < _bars.Count; index++)
                    {
                        var bar = _bars[index];
                        if (_coverage[index] != TaskbarCoverage.Clear && bar.ShownBounds.Contains(mouse)
                            && WindowAt(mouse) == bar.Handle)
                        {
                            reveal = _visualPeek = bar.Handle;
                            break;
                        }
                    }
                }
                else if (_visualPeek != IntPtr.Zero)
                {
                    var hit = WindowAt(mouse);
                    if (hit != _visualPeek && !TaskbarHelper.IsTraySurface(hit.GetClassName()))
                        obscuredPeek = _visualPeek;
                }
            }

            if (_visuals.Apply(_coverage, reveal, obscuredPeek, now))
                QueueUpdate();
        }

        private bool InspectWindow(IntPtr window, IntPtr parameter)
        {
            const int ExtendedStyle = -20;
            const int ToolWindow = 0x80;
            const int NoActivate = 0x08000000;

            if (window.IsWindowInvisible() || !TryGetVisibleBounds(window, out var bounds))
                return true;

            var windowClass = window.GetClassName();
            if (TaskbarHelper.IsDesktop(windowClass) || TaskbarHelper.IsTraySurface(windowClass) ||
                (GetWindowStyle(window, ExtendedStyle) & (ToolWindow | NoActivate)) != 0)
                return true;

            var rect = bounds.ToRectangle();
            var maximized = IsZoomed(window);
            var windowDisplay = maximized ? MonitorFromWindow(window, 2) : IntPtr.Zero;
            for (var index = 0; index < _bars.Count; index++)
            {
                var bar = _bars[index];
                var value = bar.CoverageFrom(rect, maximized, windowDisplay == bar.Monitor);
                if (value != TaskbarCoverage.Clear && (maximized || value == TaskbarCoverage.Fullscreen)
                    && !IsWindowExposed(window, Rectangle.Intersect(rect, bar.Display)))
                    continue;
                if (value > _coverage[index])
                    _coverage[index] = value;
            }
            return true;
        }

        private static bool IsWindowExposed(IntPtr window, Rectangle visible)
        {
            if (visible.Width <= 0 || visible.Height <= 0)
                return false;

            ReadOnlySpan<(int X, int Y)> samples = [(2, 2), (1, 1), (3, 1), (1, 3), (3, 3)];
            foreach (var (x, y) in samples)
                if (WindowAt(new Point(visible.Left + visible.Width * x / 4, visible.Top + visible.Height * y / 4)) == window)
                    return true;
            return false;
        }

        private void SampleVisualEdges()
        {
            if (!GetCursorPos(out var point))
                return;

            var cursor = new Point(point.x, point.y);
            if (_visualPeek != IntPtr.Zero)
            {
                if (_bars.Exists(bar => bar.Handle == _visualPeek && bar.ShownBounds.Contains(cursor)))
                    return;

                _visualPeek = IntPtr.Zero;
                _dwell = default;
                QueueUpdate();
            }

            foreach (var (bar, band) in _edges)
            {
                if (!band.Contains(cursor) || _coverage[_bars.IndexOf(bar)] == TaskbarCoverage.Clear)
                    continue;

                if (_dwell.IsReady(bar.Handle, Environment.TickCount64))
                {
                    _visualPeek = bar.Handle;
                    QueueUpdate();
                }
                return;
            }

            _dwell = default;
        }

        private static IntPtr WindowAt(Point point)
            => GetAncestor(WindowFromPhysicalPoint(new TagPoint { x = point.X, y = point.Y }), TaskbarHelper.RootWindow);

        private void SampleTaskbarEdges()
        {
            if (MenuOpen || !GetCursorPos(out var point))
                return;

            var cursor = new Point(point.x, point.y);
            var now = Environment.TickCount64;
            if (_edgeReveal is { } active)
            {
                if (active.ShownBounds.Contains(cursor))
                {
                    if (!_edgeConfirmed)
                    {
                        _edgeConfirmed = CanUseTaskbar(active, cursor);
                        if (!_edgeConfirmed && _dwell.TryRequestReveal(now))
                            TaskbarHelper.Reveal(_shell, active.Monitor);
                    }

                    return;
                }

                _dismissBar = active.Handle;
                _dismissAfter = now + EdgeDismissMs;
                ClearEdgeReveal();
            }

            foreach (var (bar, band) in _edges)
            {
                if (!band.Contains(cursor))
                    continue;

                if (!_dwell.IsReady(bar.Handle, now))
                    return;

                var shown = CanUseTaskbar(bar, cursor);
                if (shown || (_dwell.TryRequestReveal(now) && TaskbarHelper.Reveal(_shell, bar.Monitor)))
                {
                    _edgeReveal = bar;
                    _edgeConfirmed = shown;
                    _dismissBar = IntPtr.Zero;
                }

                return;
            }

            _dwell = default;
            DismissEdgeReveal(cursor, now);
        }

        private static bool CanUseTaskbar(TaskbarInfo bar, Point cursor)
            => TaskbarHelper.IsShown(bar) && WindowAt(cursor) == bar.Handle;

        private void DismissEdgeReveal(Point cursor, long now)
        {
            if (_dismissBar == IntPtr.Zero || now < _dismissAfter || MenuOpen)
                return;

            var hitClass = WindowAt(cursor).GetClassName();
            if (TaskbarHelper.IsTraySurface(hitClass) || TaskbarHelper.IsTraySurface(GetForegroundWindow().GetClassName()))
            {
                _dismissAfter = now + EdgeDismissMs;
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

        private void ClearEdgeReveal()
        {
            _dwell = default;
            _edgeReveal = null;
            _edgeConfirmed = false;
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
            var nextHide = Environment.TickCount64 + EdgeDismissMs;
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
                    nextHide = now + ExitHideRetryMs;
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
            _visuals.Dispose();
            DestroyHandle();
        }
    }
}
