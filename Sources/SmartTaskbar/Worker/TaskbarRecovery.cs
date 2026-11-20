using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using Microsoft.Win32.SafeHandles;

namespace SmartTaskbar
{
    internal sealed class TaskbarRecovery : IDisposable
    {
        internal const string WorkerArgument = "--restore-taskbar-regions";
        private const string EventPrefix = @"Local\SmartTaskbar.TaskbarRecovery.";

        private readonly Process _worker;
        private readonly EventWaitHandle _ready;
        private readonly EventWaitHandle _cancel;
        private readonly EventWaitHandle _restore;
        private readonly Dictionary<IntPtr, EventWaitHandle> _peekPromotions;
        private readonly long _deadline = Environment.TickCount64 + 3_000;

        private TaskbarRecovery(Process worker, EventWaitHandle ready, EventWaitHandle cancel, EventWaitHandle restore,
            Dictionary<IntPtr, EventWaitHandle> peekPromotions)
        {
            _worker = worker;
            _ready = ready;
            _cancel = cancel;
            _restore = restore;
            _peekPromotions = peekPromotions;
        }

        public bool IsReady => !_worker.HasExited && _ready.WaitOne(0);
        public bool IsStarting => !_worker.HasExited && Environment.TickCount64 < _deadline;
        public EventWaitHandle PeekPromotion(IntPtr window) => _peekPromotions[window];

        private static string PeekEventName(string name, string window) => name + ".Peek." + window;

        public static TaskbarRecovery? Start(IEnumerable<TaskbarInfo> bars)
        {
            if (Environment.ProcessPath is not { } path)
                return null;

            var name = EventPrefix + Guid.NewGuid().ToString("N");
            var ready = new EventWaitHandle(false, EventResetMode.ManualReset, name + ".Ready");
            var cancel = new EventWaitHandle(false, EventResetMode.ManualReset, name + ".Cancel");
            var restore = new EventWaitHandle(false, EventResetMode.ManualReset, name + ".Restore");
            var info = new ProcessStartInfo(path) { UseShellExecute = false, CreateNoWindow = true };
            if (Path.GetFileNameWithoutExtension(path).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
                info.ArgumentList.Add(Assembly.GetEntryAssembly()!.Location);

            using var parent = Process.GetCurrentProcess();
            info.ArgumentList.Add(WorkerArgument);
            info.ArgumentList.Add(parent.Id.ToString(CultureInfo.InvariantCulture));
            info.ArgumentList.Add(parent.StartTime.ToUniversalTime().Ticks.ToString(CultureInfo.InvariantCulture));
            info.ArgumentList.Add(name);
            var peekPromotions = new Dictionary<IntPtr, EventWaitHandle>();
            foreach (var bar in bars)
            {
                var window = bar.Handle.ToInt64().ToString(CultureInfo.InvariantCulture);
                info.ArgumentList.Add(window);
                peekPromotions.Add(bar.Handle, new EventWaitHandle(false, EventResetMode.ManualReset,
                    PeekEventName(name, window)));
            }

            try
            {
                if (Process.Start(info) is { } worker)
                    return new TaskbarRecovery(worker, ready, cancel, restore, peekPromotions);
            }
            catch (Win32Exception)
            {
                // Leave the native taskbars visible if a recovery process cannot be started.
            }

            ready.Dispose();
            cancel.Dispose();
            restore.Dispose();
            foreach (var promotion in peekPromotions.Values)
                promotion.Dispose();
            return null;
        }

        internal static void RunWorker(string[] args)
        {
            if (args.Length < 5 || !int.TryParse(args[1], CultureInfo.InvariantCulture, out var pid) ||
                !long.TryParse(args[2], CultureInfo.InvariantCulture, out var started) ||
                !args[3].StartsWith(EventPrefix, StringComparison.Ordinal) ||
                !Guid.TryParseExact(args[3][EventPrefix.Length..], "N", out _))
                return;

            var saved = new List<(ExplorerTaskbarRegion Region, EventWaitHandle Promotion)>();
            var workspaces = new List<TaskbarWorkspace>();
            try
            {
                using var parent = Process.GetProcessById(pid);
                if (parent.StartTime.ToUniversalTime().Ticks != started ||
                    !string.Equals(parent.MainModule?.FileName, Environment.ProcessPath, StringComparison.OrdinalIgnoreCase))
                    return;

                using var cancelled = EventWaitHandle.OpenExisting(args[3] + ".Cancel");
                using var restore = EventWaitHandle.OpenExisting(args[3] + ".Restore");
                using var ready = EventWaitHandle.OpenExisting(args[3] + ".Ready");
                using var parentExit = new EventWaitHandle(false, EventResetMode.ManualReset)
                {
                    SafeWaitHandle = new SafeWaitHandle(parent.Handle, ownsHandle: false)
                };

                Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
                var bars = new List<TaskbarInfo>();
                var shell = Fun.FindWindow(TaskbarHelper.PrimaryClass, null);
                var displays = Array.ConvertAll(Screen.AllScreens, screen => screen.Bounds);
                TaskbarHelper.ReadTaskbars(shell, displays, bars);

                for (var index = 4; index < args.Length; index++)
                {
                    if (!long.TryParse(args[index], CultureInfo.InvariantCulture, out var handle))
                        return;
                    var bar = bars.Find(bar => bar.Handle.ToInt64() == handle);
                    if (bar.Handle == IntPtr.Zero)
                        return;

                    var promotion = EventWaitHandle.OpenExisting(PeekEventName(args[3], args[index]));
                    if (ExplorerTaskbarRegion.Capture(bar.Handle) is not { } region)
                    {
                        promotion.Dispose();
                        return;
                    }
                    saved.Add((region, promotion));
                    if (TaskbarWorkspace.Capture(bar) is { } workspace)
                        workspaces.Add(workspace);
                }

                ready.Set();
                if (WaitHandle.WaitAny([cancelled, restore, parentExit]) != 0)
                {
                    if (!Fun.IsAutoHide())
                        foreach (var workspace in workspaces)
                            workspace.Restore();
                    foreach (var (region, promotion) in saved)
                    {
                        region.RestorePeek(promotion);
                        region.Restore();
                    }
                }
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or IOException or
                Win32Exception or WaitHandleCannotBeOpenedException or UnauthorizedAccessException)
            {
                // A cancelled or incomplete start leaves the main process unable to clip anything.
            }
            finally
            {
                foreach (var (region, promotion) in saved)
                {
                    region.Dispose();
                    promotion.Dispose();
                }
            }
        }

        public void RequestRestore() => _restore.Set();

        public void Dispose()
        {
            if (!_restore.WaitOne(0))
                _cancel.Set();
            _ready.Dispose();
            _restore.Dispose();
            _cancel.Dispose();
            foreach (var promotion in _peekPromotions.Values)
                promotion.Dispose();
            _worker.Dispose();
        }
    }
}
