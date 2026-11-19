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

        public Rectangle SharedEdgeBand(Rectangle otherDisplay)
        {
            var horizontal = Edge is TaskbarPosition.Bottom or TaskbarPosition.Top;
            var thickness = Math.Clamp((horizontal ? ShownBounds.Height : ShownBounds.Width) / 8, 4, 16);
            Rectangle band;
            switch (Edge)
            {
                case TaskbarPosition.Bottom when Display.Bottom == otherDisplay.Top:
                    band = Rectangle.FromLTRB(otherDisplay.Left, Display.Bottom - thickness,
                        otherDisplay.Right, Display.Bottom);
                    break;
                case TaskbarPosition.Top when Display.Top == otherDisplay.Bottom:
                    band = Rectangle.FromLTRB(otherDisplay.Left, Display.Top,
                        otherDisplay.Right, Display.Top + thickness);
                    break;
                case TaskbarPosition.Left when Display.Left == otherDisplay.Right:
                    band = Rectangle.FromLTRB(Display.Left, otherDisplay.Top,
                        Display.Left + thickness, otherDisplay.Bottom);
                    break;
                case TaskbarPosition.Right when Display.Right == otherDisplay.Left:
                    band = Rectangle.FromLTRB(Display.Right - thickness, otherDisplay.Top,
                        Display.Right, otherDisplay.Bottom);
                    break;
                default:
                    return Rectangle.Empty;
            }

            return Rectangle.Intersect(band, ShownBounds);
        }

        public bool IsCoveredBy(Rectangle window, IntPtr windowMonitor, bool maximized)
            => (!maximized || windowMonitor == Monitor) && ShownBounds.IntersectsWith(window);

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
