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
        private readonly long _deadline = Environment.TickCount64 + 3_000;

        private TaskbarRecovery(Process worker, EventWaitHandle ready, EventWaitHandle cancel, EventWaitHandle restore)
        {
            _worker = worker;
            _ready = ready;
            _cancel = cancel;
            _restore = restore;
        }

        public bool IsReady => !_worker.HasExited && _ready.WaitOne(0);
        public bool IsStarting => !_worker.HasExited && Environment.TickCount64 < _deadline;

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
            foreach (var bar in bars)
                info.ArgumentList.Add(bar.Handle.ToInt64().ToString(CultureInfo.InvariantCulture));

            try
            {
                if (Process.Start(info) is { } worker)
                    return new TaskbarRecovery(worker, ready, cancel, restore);
            }
            catch (Win32Exception)
            {
                // Leave the native taskbars visible if a recovery process cannot be started.
            }

            ready.Dispose();
            cancel.Dispose();
            restore.Dispose();
            return null;
        }

        internal static void RunWorker(string[] args)
        {
            if (args.Length < 5 || !int.TryParse(args[1], CultureInfo.InvariantCulture, out var pid) ||
                !long.TryParse(args[2], CultureInfo.InvariantCulture, out var started) ||
                !args[3].StartsWith(EventPrefix, StringComparison.Ordinal) ||
                !Guid.TryParseExact(args[3][EventPrefix.Length..], "N", out _))
                return;

            var saved = new List<ExplorerTaskbarRegion>();
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
                for (var index = 4; index < args.Length; index++)
                {
                    if (!long.TryParse(args[index], CultureInfo.InvariantCulture, out var handle)
                        || ExplorerTaskbarRegion.Capture(new IntPtr(handle)) is not { } region)
                        return;
                    saved.Add(region);
                }

                ready.Set();
                if (WaitHandle.WaitAny([cancelled, restore, parentExit]) != 0)
                    foreach (var region in saved)
                        region.Restore();
            }
            catch (Exception error) when (error is ArgumentException or InvalidOperationException or IOException or
                Win32Exception or WaitHandleCannotBeOpenedException or UnauthorizedAccessException)
            {
                // A cancelled or incomplete start leaves the main process unable to clip anything.
            }
            finally
            {
                foreach (var region in saved)
                    region.Dispose();
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
            _worker.Dispose();
        }
    }
}
