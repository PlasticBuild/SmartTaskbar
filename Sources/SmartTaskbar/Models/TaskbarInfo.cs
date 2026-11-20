using System.Drawing;

namespace SmartTaskbar
{
    internal readonly record struct TaskbarInfo(
        IntPtr Handle, IntPtr Monitor, Rectangle Display, Rectangle ShownBounds, TaskbarPosition Edge)
    {
        public bool IsShown(Rectangle actual)
            => VisibleThickness(actual) >
                (Edge is TaskbarPosition.Bottom or TaskbarPosition.Top ? ShownBounds.Height : ShownBounds.Width) / 2;

        public int VisibleThickness(Rectangle actual)
        {
            var visible = Rectangle.Intersect(actual, Display);
            if (visible.Width <= 0 || visible.Height <= 0)
                return 0;

            return Edge is TaskbarPosition.Bottom or TaskbarPosition.Top
                ? visible.Height : visible.Width;
        }

        public Rectangle RevealBand()
        {
            var depth = Edge is TaskbarPosition.Bottom or TaskbarPosition.Top
                ? ShownBounds.Height : ShownBounds.Width;
            var thickness = Math.Min(depth, Math.Clamp(depth / 3, 16, 48));
            var bar = ShownBounds;
            return Edge switch
            {
                TaskbarPosition.Bottom => Rectangle.FromLTRB(bar.Left, bar.Bottom - thickness, bar.Right, bar.Bottom),
                TaskbarPosition.Top => Rectangle.FromLTRB(bar.Left, bar.Top, bar.Right, bar.Top + thickness),
                TaskbarPosition.Left => Rectangle.FromLTRB(bar.Left, bar.Top, bar.Left + thickness, bar.Bottom),
                TaskbarPosition.Right => Rectangle.FromLTRB(bar.Right - thickness, bar.Top, bar.Right, bar.Bottom),
                _ => Rectangle.Empty
            };
        }

        public TaskbarCoverage CoverageFrom(Rectangle window, bool maximized, bool activeOnDisplay)
        {
            if (window.Contains(Display))
                return TaskbarCoverage.Fullscreen;

            if (maximized)
                return activeOnDisplay ? TaskbarCoverage.Overlap : TaskbarCoverage.Clear;

            return ShownBounds.IntersectsWith(window) ? TaskbarCoverage.Overlap : TaskbarCoverage.Clear;
        }

        public static Rectangle Dock(Rectangle actual, Rectangle display, TaskbarPosition edge)
        {
            switch (edge)
            {
                case TaskbarPosition.Bottom:
                    actual.Y = display.Bottom - actual.Height;
                    break;
                case TaskbarPosition.Top:
                    actual.Y = display.Top;
                    break;
                case TaskbarPosition.Left:
                    actual.X = display.Left;
                    break;
                case TaskbarPosition.Right:
                    actual.X = display.Right - actual.Width;
                    break;
            }

            return Rectangle.Intersect(actual, display);
        }
    }
}
