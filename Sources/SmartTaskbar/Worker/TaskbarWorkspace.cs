namespace SmartTaskbar
{
    using static Fun;

    internal sealed class TaskbarWorkspace(TaskbarInfo bar, Rectangle original, Rectangle expanded)
    {
        private const uint SpiSetWorkArea = 0x002F;
        private const uint ResizeFlags = 0x4000 | 0x0400 | 0x0200 | 0x0010 | 0x0004;

        public static TaskbarWorkspace? Capture(TaskbarInfo bar)
        {
            var info = MonitorInfo.Create();
            if (!GetMonitorInfo(bar.Monitor, ref info) || info.Display.ToRectangle() != bar.Display)
                return null;

            var original = info.WorkArea.ToRectangle();
            var expanded = bar.ReclaimTaskbarStrip(original);
            return original == expanded ? null : new TaskbarWorkspace(bar, original, expanded);
        }

        public bool Expand() => MoveEdge(original, expanded);
        public bool Restore() => MoveEdge(expanded, original);

        private bool MoveEdge(Rectangle from, Rectangle to)
        {
            var info = MonitorInfo.Create();
            if (!GetMonitorInfo(bar.Monitor, ref info) || info.Display.ToRectangle() != bar.Display
                || info.WorkArea.ToRectangle() != from)
                return true;

            // Do not broadcast a desktop layout change to Explorer. Maximized apps still use
            // the new work area, and Explorer keeps the existing desktop icon coordinates.
            var native = TagRect.FromRectangle(to);
            if (!SetWorkArea(SpiSetWorkArea, 0, ref native, 0))
                return false;
            if (!GetMonitorInfo(bar.Monitor, ref info) || info.WorkArea.ToRectangle() != to)
                return false;

            EnumWindows((window, _) =>
            {
                if (!IsZoomed(window) || window.IsWindowInvisible() || MonitorFromWindow(window, 0) != bar.Monitor
                    || !TryGetVisibleBounds(window, out var visible) || !GetWindowRect(window, out var raw))
                    return true;

                var target = raw.ToRectangle();
                var matched = true;
                switch (bar.Edge)
                {
                    case TaskbarPosition.Bottom when visible.bottom == from.Bottom:
                        target.Height += to.Bottom - from.Bottom;
                        break;
                    case TaskbarPosition.Top when visible.top == from.Top:
                        target = Rectangle.FromLTRB(target.Left, target.Top + to.Top - from.Top, target.Right, target.Bottom);
                        break;
                    case TaskbarPosition.Left when visible.left == from.Left:
                        target = Rectangle.FromLTRB(target.Left + to.Left - from.Left, target.Top, target.Right, target.Bottom);
                        break;
                    case TaskbarPosition.Right when visible.right == from.Right:
                        target.Width += to.Right - from.Right;
                        break;
                    default:
                        matched = false;
                        break;
                }

                if (matched && target.Width > 0 && target.Height > 0)
                    SetWindowPos(window, IntPtr.Zero, target.X, target.Y, target.Width, target.Height, ResizeFlags);
                return true;
            }, IntPtr.Zero);
            return true;
        }
    }
}
