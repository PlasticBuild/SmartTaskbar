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

        public Rectangle LimitRevealBand(Rectangle band, RevealZone zone, Rectangle? island)
        {
            var horizontal = Edge is TaskbarPosition.Top or TaskbarPosition.Bottom;
            if (zone == RevealZone.VisibleTaskbar && island is { } shape)
            {
                var limited = Rectangle.Intersect(band, horizontal
                    ? new Rectangle(shape.X, band.Y, shape.Width, band.Height)
                    : new Rectangle(band.X, shape.Y, band.Width, shape.Height));
                return limited.IsEmpty ? band : limited;
            }
            if (zone != RevealZone.CenterHalf)
                return band;

            if (horizontal)
            {
                var width = Math.Max(1, band.Width / 2);
                return new Rectangle(band.X + (band.Width - width) / 2, band.Y, width, band.Height);
            }
            var height = Math.Max(1, band.Height / 2);
            return new Rectangle(band.X, band.Y + (band.Height - height) / 2, band.Width, height);
        }

        public TaskbarCoverage CoverageFrom(Rectangle window, bool maximized, bool fillsOwnDisplay)
        {
            if (maximized && !fillsOwnDisplay)
                return TaskbarCoverage.Clear;

            if (window.Contains(Display))
                return TaskbarCoverage.Fullscreen;

            return maximized || ShownBounds.IntersectsWith(window) ? TaskbarCoverage.Overlap : TaskbarCoverage.Clear;
        }

        public Rectangle ReclaimTaskbarStrip(Rectangle work)
        {
            if (!Display.Contains(work))
                return work;

            // Other reserved edges may belong to another Windows appbar.
            return Edge switch
            {
                TaskbarPosition.Bottom when work.Bottom == ShownBounds.Top =>
                    Rectangle.FromLTRB(work.Left, work.Top, work.Right, Display.Bottom),
                TaskbarPosition.Top when work.Top == ShownBounds.Bottom =>
                    Rectangle.FromLTRB(work.Left, Display.Top, work.Right, work.Bottom),
                TaskbarPosition.Left when work.Left == ShownBounds.Right =>
                    Rectangle.FromLTRB(Display.Left, work.Top, work.Right, work.Bottom),
                TaskbarPosition.Right when work.Right == ShownBounds.Left =>
                    Rectangle.FromLTRB(work.Left, work.Top, Display.Right, work.Bottom),
                _ => work
            };
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
