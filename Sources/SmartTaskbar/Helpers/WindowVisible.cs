namespace SmartTaskbar
{
    public static partial class Fun
    {
        private const int TrayDwmWaCloaked = 14;

        public static bool IsWindowInvisible(this IntPtr handle)
        {
            if (!IsWindowVisible(handle) || IsIconic(handle)) return true;

            return DwmGetWindowAttribute(handle, TrayDwmWaCloaked, out bool cloaked, sizeof(int)) == 0 && cloaked;
        }

        internal static bool TryGetVisibleBounds(IntPtr handle, out TagRect rect)
            => DwmGetWindowAttribute(handle, 9, out rect, sizeof(int) * 4) == 0 || GetWindowRect(handle, out rect);
    }
}
