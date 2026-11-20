using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar
{
    internal sealed class TaskbarVisuals : IDisposable
    {
        private sealed class Tray(TaskbarInfo bar)
        {
            public TaskbarInfo Bar { get; } = bar;
            public int Depth { get; } = bar.Edge is TaskbarPosition.Top or TaskbarPosition.Bottom
                ? bar.ShownBounds.Height : bar.ShownBounds.Width;
            public ExplorerTaskbarRegion? Region { get; set; }
            public TaskbarWorkspace? Workspace { get; set; }
            public TaskbarRetraction Animation = new();
            public int Pixels = -1;
            public bool Clipped;
        }

        private readonly Timer _frames = new() { Interval = 16 };
        private Tray[] _trays = [];
        private TaskbarRecovery? _recovery;
        private long _retryAfter;

        public TaskbarVisuals()
        {
            _frames.Tick += (_, _) => Render(Environment.TickCount64);
        }

        public void SetLayout(List<TaskbarInfo> bars)
        {
            if (_trays.Select(tray => tray.Bar).SequenceEqual(bars))
                return;

            Reset();
            _trays = bars.Select(bar => new Tray(bar)).ToArray();
        }

        public Rectangle? OpenIslandBounds(IntPtr window)
            => Array.Find(_trays, tray => tray.Bar.Handle == window)?.Region?.OpenIslandBounds;

        public void RefreshOpenShapes()
        {
            foreach (var tray in _trays)
                if (!tray.Clipped)
                    tray.Region?.RefreshOpenShape();
        }

        // Returns true while the recovery process is still starting.
        public bool Apply(TaskbarCoverage[] coverage, IntPtr revealed, IntPtr obscuredPeek, long now)
        {
            if (now < _retryAfter || _trays.Length != coverage.Length || _trays.Length == 0)
                return false;

            if (_recovery is null)
            {
                foreach (var tray in _trays)
                {
                    if (ExplorerTaskbarRegion.RecoverIfEmpty(tray.Bar.Handle))
                        tray.Region = ExplorerTaskbarRegion.Capture(tray.Bar.Handle);
                    if (tray.Region is null)
                    {
                        CancelChanges();
                        return false;
                    }
                    tray.Workspace = TaskbarWorkspace.Capture(tray.Bar);
                }
                _recovery = TaskbarRecovery.Start(_trays.Select(tray => tray.Bar));
                if (_recovery is null)
                {
                    CancelChanges();
                    return false;
                }
            }

            if (!_recovery.IsReady)
            {
                if (_recovery.IsStarting && !_trays.Any(tray => tray.Clipped))
                    return true;

                CancelChanges();
                return false;
            }

            for (var index = 0; index < _trays.Length; index++)
            {
                var tray = _trays[index];
                if (tray.Workspace is { } workspace && !workspace.Expand())
                {
                    CancelChanges();
                    return false;
                }

                var covered = coverage[index] != TaskbarCoverage.Clear;
                var visible = !covered || tray.Bar.Handle == revealed;
                var promotion = _recovery.PeekPromotion(tray.Bar.Handle);
                if (!covered)
                {
                    // Explorer can restore the tray's normal stacking when fullscreen ends.
                    promotion.Reset();
                }
                else if (tray.Bar.Handle == obscuredPeek)
                {
                    tray.Region!.RaiseForPeek(promotion);
                }
                if (visible && !tray.Clipped && tray.Region!.IsEmpty() && !tray.Region.Restore())
                {
                    CancelChanges();
                    return false;
                }
                if (visible && tray.Clipped && tray.Pixels == 0 && !tray.Region!.IsEmpty())
                {
                    // Explorer has already brought the tray back, such as when leaving a fullscreen app.
                    // Do not clip it a second time just to replay the animation.
                    tray.Animation = new TaskbarRetraction();
                    tray.Pixels = tray.Depth;
                    tray.Clipped = false;
                }
                tray.Animation.SetVisible(visible, now);

                // Explorer may replace the region after a settings or display change.
                if (!visible && tray.Clipped && tray.Pixels == 0 && !tray.Region!.IsEmpty())
                    tray.Pixels = -1;
            }

            Render(now);
            return false;
        }

        private void Render(long now)
        {
            if (_recovery is null || !_recovery.IsReady)
            {
                CancelChanges();
                return;
            }

            var animating = false;
            foreach (var tray in _trays)
            {
                var moving = tray.Animation.IsAnimating(now);
                animating |= moving;
                var pixels = (int)Math.Round(tray.Animation.FractionAt(now) * tray.Depth);
                if (pixels >= tray.Depth)
                {
                    if (tray.Clipped && !tray.Region!.Restore(onlyIfStillClipped: true))
                    {
                        CancelChanges();
                        return;
                    }
                    tray.Clipped = false;
                }
                else if (!tray.Clipped || pixels != tray.Pixels)
                {
                    if (!tray.Region!.Clip(tray.Bar.Edge, pixels))
                    {
                        CancelChanges();
                        return;
                    }
                    tray.Clipped = true;
                }
                tray.Pixels = pixels;
                if (pixels == 0 && !moving && !tray.Region!.RestorePeek(_recovery.PeekPromotion(tray.Bar.Handle)))
                {
                    CancelChanges();
                    return;
                }
            }
            _frames.Enabled = animating;
        }

        private void CancelChanges()
        {
            Reset();
            // Explorer can still be creating its taskbar windows. The watchdog will try again
            // after the shell has settled, with the native taskbars visible in the meantime.
            _retryAfter = Environment.TickCount64 + 5_000;
        }

        public void Reset()
        {
            _frames.Stop();
            var restoreWorkArea = _recovery is not null && !Fun.IsAutoHide();
            foreach (var tray in _trays)
            {
                if (restoreWorkArea && tray.Workspace is { } workspace && !workspace.Restore())
                    _recovery?.RequestRestore();
                tray.Workspace = null;

                if (tray.Region is { } region)
                {
                    if (_recovery is not null && !region.RestorePeek(_recovery.PeekPromotion(tray.Bar.Handle)))
                        _recovery.RequestRestore();
                    if (tray.Clipped && !region.Restore(onlyIfStillClipped: true))
                        _recovery?.RequestRestore();
                    region.Dispose();
                    tray.Region = null;
                }
                tray.Clipped = false;
                tray.Pixels = -1;
                tray.Animation = new TaskbarRetraction();
            }
            _recovery?.Dispose();
            _recovery = null;
            _retryAfter = 0;
        }

        public void Dispose()
        {
            Reset();
            _frames.Dispose();
        }
    }
}
