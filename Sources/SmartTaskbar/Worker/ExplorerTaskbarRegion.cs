namespace SmartTaskbar
{
    using static Fun;

    internal sealed class ExplorerTaskbarRegion(IntPtr window, int processId, IntPtr original) : IDisposable
    {
        private const int NoRegion = 0;
        private const int EmptyRegion = 1;
        private const int IntersectRegions = 1;
        private const int CopyRegion = 5;
        private const int ExtendedStyle = -20;
        private const int TopmostStyle = 0x8;
        private const uint NoSize = 0x1;
        private const uint NoMove = 0x2;
        private const uint NoActivate = 0x10;
        private const uint NoOwnerZOrder = 0x200;
        private const uint ReorderFlags = NoSize | NoMove | NoActivate | NoOwnerZOrder;
        private static readonly IntPtr Topmost = new(-1);
        private static readonly IntPtr NotTopmost = new(-2);

        private IntPtr _original = original;

        public static ExplorerTaskbarRegion? Capture(IntPtr window)
        {
            if (window.GetClassName() is not (TaskbarHelper.PrimaryClass or "Shell_SecondaryTrayWnd") ||
                GetWindowThreadProcessId(window, out var processId) == 0)
                return null;

            var original = CreateRectRgn(0, 0, 0, 0);
            if (original == IntPtr.Zero)
                return null;

            var type = GetWindowRgn(window, original);
            if (type > EmptyRegion)
                return new ExplorerTaskbarRegion(window, processId, original);

            DeleteObject(original);
            return type == NoRegion && IsWindowVisible(window)
                ? new ExplorerTaskbarRegion(window, processId, IntPtr.Zero)
                : null;
        }

        public bool Clip(TaskbarPosition edge, int visiblePixels)
        {
            if (!IsSameWindow() || !GetWindowRect(window, out var raw))
                return false;

            var bounds = raw.ToRectangle();
            var rect = TaskbarRetraction.ClipBounds(bounds.Size, edge, visiblePixels);
            var region = CreateRectRgn(rect.Left, rect.Top, rect.Right, rect.Bottom);
            if (region == IntPtr.Zero)
                return false;
            if (_original != IntPtr.Zero && CombineRgn(region, region, _original, IntersectRegions) == NoRegion)
            {
                DeleteObject(region);
                return false;
            }
            return SetRegion(region);
        }

        public bool IsEmpty() => GetWindowRgnBox(window, out _) == EmptyRegion;

        public void RaiseForPeek(EventWaitHandle promotion)
        {
            if (!IsSameWindow())
                return;

            // Record a change of band before touching Explorer so the recovery process can undo it.
            if (!IsTopmost())
                promotion.Set();
            SetWindowPos(window, Topmost, 0, 0, 0, 0, ReorderFlags);
        }

        public bool RestorePeek(EventWaitHandle promotion)
        {
            if (!promotion.WaitOne(0))
                return true;
            if (IsSameWindow() && IsTopmost() && !SetWindowPos(window, NotTopmost, 0, 0, 0, 0, ReorderFlags))
                return false;

            promotion.Reset();
            return true;
        }

        public bool Restore()
        {
            if (!IsSameWindow())
                return true;
            if (_original == IntPtr.Zero)
                return SetRegion(IntPtr.Zero);

            var region = CreateRectRgn(0, 0, 0, 0);
            if (region == IntPtr.Zero)
                return false;
            if (CombineRgn(region, _original, IntPtr.Zero, CopyRegion) == NoRegion)
            {
                DeleteObject(region);
                return false;
            }
            return SetRegion(region);
        }

        private bool IsSameWindow()
            => GetWindowThreadProcessId(window, out var current) != 0 && current == processId &&
                window.GetClassName() is TaskbarHelper.PrimaryClass or "Shell_SecondaryTrayWnd";

        private bool IsTopmost() => (GetWindowStyle(window, ExtendedStyle) & TopmostStyle) != 0;

        private bool SetRegion(IntPtr region)
        {
            if (SetWindowRgn(window, region, true) != 0)
                return true; // Windows takes ownership after a successful change.
            if (region != IntPtr.Zero)
                DeleteObject(region);
            return false;
        }

        public void Dispose()
        {
            if (_original != IntPtr.Zero)
                DeleteObject(_original);
            _original = IntPtr.Zero;
        }
    }
}
