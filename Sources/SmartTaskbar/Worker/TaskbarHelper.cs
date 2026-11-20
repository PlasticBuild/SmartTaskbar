using System.Diagnostics;

namespace SmartTaskbar
{
    using static Fun;

    internal static class TaskbarHelper
    {
        internal const string PrimaryClass = "Shell_TrayWnd";
        internal const string SecondaryClass = "Shell_SecondaryTrayWnd";
        internal const int ConvertibleSlateMode = 0x2003;
        private const uint GetTaskbarPosition = 5;
        private const uint GetAutoHideBarEx = 11;
        private const uint TrayAutoHideMessage = 0x05D1;
        internal const uint RootWindow = 2;

        // Explorer can report other edges. Prefer bottom when an unregistered bar straddles two displays.
        private static readonly TaskbarPosition[] FallbackEdges =
            [TaskbarPosition.Bottom, TaskbarPosition.Left, TaskbarPosition.Top, TaskbarPosition.Right];

        public static void ReadTaskbars(IntPtr shell, Rectangle[] displays, List<TaskbarInfo> taskbars)
        {
            taskbars.Clear();
            if (shell == IntPtr.Zero)
                return;

            var primary = AppBar(shell);
            if (SHAppBarMessage(GetTaskbarPosition, ref primary) != IntPtr.Zero
                && primary.uEdge <= (uint)TaskbarPosition.Bottom)
            {
                var bounds = primary.rc.ToRectangle();
                foreach (var display in displays)
                {
                    if (display.Contains(CenterPoint(bounds)))
                    {
                        Add(shell, bounds, display, (TaskbarPosition)primary.uEdge, taskbars);
                        break;
                    }
                }
            }
            if (taskbars.Count == 0)
                AddFromWindow(shell, displays, taskbars);

            var handle = IntPtr.Zero;
            while ((handle = FindWindowEx(IntPtr.Zero, handle, SecondaryClass, null)) != IntPtr.Zero)
            {
                if (IsWindowVisible(handle))
                    AddSecondary(handle, displays, taskbars);
            }
        }

        private static void AddSecondary(IntPtr handle, Rectangle[] displays, List<TaskbarInfo> taskbars)
        {
            if (!GetWindowRect(handle, out var rect))
                return;

            // A hidden taskbar can lie mostly on the adjacent display. Ask Explorer which edge owns it.
            foreach (var display in displays)
            {
                if (HasTaskbar(display, taskbars))
                    continue;

                var appbar = AppBar();
                appbar.rc = TagRect.FromRectangle(display);
                foreach (var edge in FallbackEdges)
                {
                    appbar.uEdge = (uint)edge;
                    if (SHAppBarMessage(GetAutoHideBarEx, ref appbar) != handle)
                        continue;

                    Add(handle, rect.ToRectangle(), display, edge, taskbars);
                    return;
                }
            }

            AddFromWindow(handle, displays, taskbars);
        }

        private static void AddFromWindow(IntPtr handle, Rectangle[] displays, List<TaskbarInfo> taskbars)
        {
            if (!GetWindowRect(handle, out var rect))
                return;

            var actual = rect.ToRectangle();
            // Before registration settles, match the window to its docked edge or its hidden strip.
            foreach (var edge in FallbackEdges)
            {
                foreach (var display in displays)
                {
                    if (!HasTaskbar(display, taskbars) && FitsEdge(actual, display, edge))
                    {
                        Add(handle, actual, display, edge, taskbars);
                        return;
                    }
                }
            }
        }

        private static bool FitsEdge(Rectangle actual, Rectangle display, TaskbarPosition edge)
        {
            const int tolerance = 4;
            return edge switch
            {
                TaskbarPosition.Bottom => NearlyEqual(actual.Left, display.Left)
                    && NearlyEqual(actual.Right, display.Right)
                    && actual.Bottom >= display.Bottom - tolerance && actual.Top <= display.Bottom + tolerance,
                TaskbarPosition.Top => NearlyEqual(actual.Left, display.Left)
                    && NearlyEqual(actual.Right, display.Right)
                    && actual.Top <= display.Top + tolerance && actual.Bottom >= display.Top - tolerance,
                TaskbarPosition.Left => NearlyEqual(actual.Top, display.Top)
                    && NearlyEqual(actual.Bottom, display.Bottom)
                    && actual.Left <= display.Left + tolerance && actual.Right >= display.Left - tolerance,
                TaskbarPosition.Right => NearlyEqual(actual.Top, display.Top)
                    && NearlyEqual(actual.Bottom, display.Bottom)
                    && actual.Right >= display.Right - tolerance && actual.Left <= display.Right + tolerance,
                _ => false
            };

            static bool NearlyEqual(int first, int second) => Math.Abs(first - second) <= tolerance;
        }

        private static bool HasTaskbar(Rectangle display, List<TaskbarInfo> bars)
        {
            foreach (var bar in bars)
                if (bar.Display == display)
                    return true;
            return false;
        }

        private static void Add(IntPtr handle, Rectangle actual, Rectangle display, TaskbarPosition edge,
            List<TaskbarInfo> taskbars)
        {
            var docked = TaskbarInfo.Dock(actual, display, edge);
            var monitor = MonitorFromPoint(Center(display), 0);
            if (docked.Width > 0 && docked.Height > 0 && monitor != IntPtr.Zero)
                taskbars.Add(new TaskbarInfo(handle, monitor, display, docked, edge));
        }

        private static Point CenterPoint(Rectangle rect)
            => new(rect.Left + rect.Width / 2, rect.Top + rect.Height / 2);

        private static TagPoint Center(Rectangle rect)
        {
            var center = CenterPoint(rect);
            return new TagPoint { x = center.X, y = center.Y };
        }

        public static bool IsShown(TaskbarInfo bar)
            => GetWindowRect(bar.Handle, out var actual) && bar.IsShown(actual.ToRectangle());

        public static bool IsBeyondAutoHideStrip(TaskbarInfo bar)
            => GetWindowRect(bar.Handle, out var actual) && bar.VisibleThickness(actual.ToRectangle()) > 4;

        // Explorer routes reveals through the primary taskbar, with the intended monitor as lParam.
        public static bool Reveal(IntPtr shell, IntPtr monitor)
            => PostMessage(shell, TrayAutoHideMessage, new IntPtr(1), monitor);

        // Explorer's hide command acts on the active tray and provides no reliable monitor selector.
        public static void Hide(IntPtr shell)
            => PostMessage(shell, TrayAutoHideMessage, IntPtr.Zero, IntPtr.Zero);

        public static bool IsDesktop(string windowClass) => windowClass is "Progman" or "WorkerW";

        public static bool IsTraySurface(string windowClass) => windowClass is PrimaryClass or SecondaryClass
            or "Windows.UI.Core.CoreWindow" or "XamlExplorerHostIslandWindow" || IsFlyoutClass(windowClass);

        private static bool IsFlyoutClass(string windowClass) => windowClass is
            "TopLevelWindowForOverflowXamlIsland" or "NotifyIconOverflowWindow" or "TaskListThumbnailWnd"
            or "TaskListOverlayWnd" or "ControlCenterWindow" or "NotificationCenterWindow";

        public static bool IsTaskbarThumbnail(IntPtr window, int shellProcess)
            => shellProcess != 0 && window.GetClassName() is ("TaskListThumbnailWnd" or "TaskListOverlayWnd")
                && GetWindowThreadProcessId(window, out var process) != 0 && process == shellProcess;

        public static bool IsVisibleFlyout(IntPtr window, string windowClass, int shellProcess)
        {
            if (IsFlyoutClass(windowClass))
                return true;

            if (shellProcess == 0 || windowClass is not ("#32768" or "Xaml_WindowedPopupClass" or "XamlExplorerHostIslandWindow") ||
                GetWindowThreadProcessId(window, out var ownerProcess) == 0 || ownerProcess != shellProcess)
                return false;
            // Tray tooltips may be owned by the taskbar too, but don't participate in menu focus.
            const int ExtendedStyle = -20;
            const int NoActivate = 0x08000000;
            if (windowClass == "Xaml_WindowedPopupClass" && (GetWindowStyle(window, ExtendedStyle) & NoActivate) != 0)
                return false;

            const uint OwnerWindow = 4;
            var owner = GetWindow(window, OwnerWindow);
            var ownerClass = owner.GetClassName();
            return ownerClass is PrimaryClass or SecondaryClass ||
                IsFlyoutClass(ownerClass) && !owner.IsWindowInvisible();
        }

        public static bool IsForegroundShellFlyout(IntPtr window, int shellProcess)
        {
            var windowClass = window.GetClassName();
            if (windowClass is not ("Windows.UI.Core.CoreWindow" or "XamlExplorerHostIslandWindow" or
                "ControlCenterWindow" or "NotificationCenterWindow") || !IsWindowVisible(window) ||
                    GetWindowThreadProcessId(window, out var processId) == 0)
                return false;
            if (processId == shellProcess)
                return windowClass is "Windows.UI.Core.CoreWindow" or "XamlExplorerHostIslandWindow";
            try
            {
                using var process = Process.GetProcessById(processId);
                return process.ProcessName is "ShellHost" or "ShellExperienceHost" or "StartMenuExperienceHost" or "SearchHost";
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or System.ComponentModel.Win32Exception)
            {
                return false;
            }
        }
    }
}
