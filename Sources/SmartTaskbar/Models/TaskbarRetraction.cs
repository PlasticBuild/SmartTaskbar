using System.Drawing;

namespace SmartTaskbar
{
    internal struct TaskbarRetraction
    {
        private const int DurationMs = 240;
        private double _from;
        private double _to;
        private long _startedAt;

        public TaskbarRetraction()
        {
            _from = 1;
            _to = 1;
        }

        public void SetVisible(bool visible, long now)
        {
            var target = visible ? 1 : 0;
            if (_to == target)
                return;
            _from = FractionAt(now);
            _to = target;
            _startedAt = now;
        }

        public double FractionAt(long now)
        {
            var time = Math.Clamp((now - _startedAt) / (double)DurationMs, 0, 1);
            var eased = time * time * (3 - 2 * time);
            return _from + (_to - _from) * eased;
        }

        public bool IsAnimating(long now) => _from != _to && now - _startedAt < DurationMs;

        public static Rectangle ClipBounds(Size size, TaskbarPosition edge, int visible)
        {
            if (visible <= 0)
                return Rectangle.Empty;
            return edge switch
            {
                TaskbarPosition.Bottom => Rectangle.FromLTRB(0, Math.Max(0, size.Height - visible), size.Width, size.Height),
                TaskbarPosition.Top => new Rectangle(0, 0, size.Width, Math.Min(visible, size.Height)),
                TaskbarPosition.Right => Rectangle.FromLTRB(Math.Max(0, size.Width - visible), 0, size.Width, size.Height),
                _ => new Rectangle(0, 0, Math.Min(visible, size.Width), size.Height)
            };
        }
    }
}
