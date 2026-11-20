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
            public TaskbarRetraction Animation = new();
            public int Pixels = -1;
            public bool Clipped;
        }

        private readonly Timer _frames = new() { Interval = 16 };
        private Tray[] _trays = [];
        private TaskbarRecovery? _recovery;
        private bool _failed;

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

        // Returns true while the recovery process is still starting.
        public bool Apply(TaskbarCoverage[] coverage, IntPtr revealed, bool showAll, long now)
        {
            if (_failed || _trays.Length != coverage.Length || _trays.Length == 0)
                return false;

            if (_recovery is null)
            {
                if (showAll || Array.TrueForAll(coverage, value => value == TaskbarCoverage.Clear))
                    return false;

                foreach (var tray in _trays)
                {
                    tray.Region = ExplorerTaskbarRegion.Capture(tray.Bar.Handle);
                    if (tray.Region is null)
                    {
                        CancelChanges();
                        return false;
                    }
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
                var visible = showAll || coverage[index] == TaskbarCoverage.Clear || tray.Bar.Handle == revealed;
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
                animating |= tray.Animation.IsAnimating(now);
                var pixels = (int)Math.Round(tray.Animation.FractionAt(now) * tray.Depth);
                if (pixels >= tray.Depth)
                {
                    if (tray.Clipped && !tray.Region!.Restore())
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
            }
            _frames.Enabled = animating;
        }

        private void CancelChanges()
        {
            Reset();
            _failed = true;
        }

        public void Reset()
        {
            _frames.Stop();
            foreach (var tray in _trays)
            {
                if (tray.Region is { } region)
                {
                    if (tray.Clipped && !region.Restore())
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
            _failed = false;
        }

        public void Dispose()
        {
            Reset();
            _frames.Dispose();
        }
    }
}
