using System.Runtime.InteropServices;

namespace SmartTaskbar
{
    public static partial class Fun
    {
        private const int AutoHide = 1;
        private const uint GetState = 4;
        private const uint SetState = 10;

        internal static AppbarData AppBar(IntPtr shell = default)
            => new() { cbSize = (uint)Marshal.SizeOf<AppbarData>(), hWnd = shell };

        public static bool IsAutoHide()
        {
            var data = AppBar();
            return (SHAppBarMessage(GetState, ref data).ToInt64() & AutoHide) != 0;
        }

        public static void SetAutoHide(bool enabled)
        {
            var shell = FindWindow(TaskbarHelper.PrimaryClass, null);
            if (shell == IntPtr.Zero || enabled == IsAutoHide())
                return;

            var data = AppBar(shell);
            data.lParam = (IntPtr)(enabled ? AutoHide : 2);
            SHAppBarMessage(SetState, ref data);
        }
    }
}
