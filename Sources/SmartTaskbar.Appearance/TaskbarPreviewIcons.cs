using System.Runtime.InteropServices;
using System.Windows.Automation;
using Microsoft.Win32;

namespace SmartTaskbar.Appearance;

internal sealed record PreviewIcon(string Name, Bitmap Image);

internal sealed record TaskbarPreviewIcons(string Signature, PreviewIcon[] Icons, int SearchMode) : IDisposable
{
    private const string AppIdPrefix = "Appid: ";
    private const uint ShellSystemIconIndex = 0x4000;
    private const uint ShellPidl = 0x8;

    public static TaskbarPreviewIcons? Read(string? previous)
    {
        var pins = Directory.Exists(PinnedFolder)
            ? Directory.GetFiles(PinnedFolder, "*.lnk")
            : [];
        var items = ReadTaskbar();
        if (items.Count == 0)
        {
            if (previous is { Length: > 0 })
                return null; // The tray can be between layouts or restarted briefly.
            items.AddRange(pins.Select(path => (path, Path.GetFileNameWithoutExtension(path))));
        }
        using var searchKey = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Search");
        var search = searchKey?.GetValue("SearchboxTaskbarMode") as int? ?? 0;
        var signature = search + "\n" + string.Join('\n', items.Select(item => item.Id));
        if (signature == previous)
            return null;

        var icons = new List<PreviewIcon>();
        foreach (var (id, name) in items)
        {
            var pin = Array.Find(pins, path => MatchesPin(name, Path.GetFileNameWithoutExtension(path)));
            var image = ReadIcon(pin) ?? ReadIcon(ResolvePath(id)) ?? ReadIcon(@"shell:AppsFolder\" + id);
            icons.Add(new PreviewIcon(name, image ?? SystemIcons.Application.ToBitmap()));
        }
        return new TaskbarPreviewIcons(signature, [.. icons], search);
    }

    private static bool MatchesPin(string name, string shortcut)
        => name.StartsWith(shortcut, StringComparison.OrdinalIgnoreCase) &&
            (name.Length == shortcut.Length || name[shortcut.Length] is ',' or ':' or ' ' or '(');

    private static List<(string Id, string Name)> ReadTaskbar()
    {
        var items = new List<(string Id, string Name)>();
        foreach (var className in new[] { "Shell_TrayWnd", "Shell_SecondaryTrayWnd" })
        {
            var window = FindWindow(className, null);
            if (window == IntPtr.Zero)
                continue;
            try
            {
                var tray = AutomationElement.FromHandle(window);
                var properties = new CacheRequest { AutomationElementMode = AutomationElementMode.None };
                properties.Add(AutomationElement.AutomationIdProperty);
                properties.Add(AutomationElement.NameProperty);
                using (properties.Activate())
                {
                    var buttons = tray.FindAll(TreeScope.Descendants,
                        new PropertyCondition(AutomationElement.ClassNameProperty, "Taskbar.TaskListButtonAutomationPeer"));
                    foreach (AutomationElement node in buttons)
                    {
                        var id = node.Cached.AutomationId;
                        if (id.StartsWith(AppIdPrefix, StringComparison.OrdinalIgnoreCase))
                            items.Add((id[AppIdPrefix.Length..], node.Cached.Name));
                    }
                }
            }
            catch (Exception error) when (error is COMException or InvalidOperationException or ArgumentException)
            {
                items.Clear();
            }
            if (items.Count > 0)
                break;
        }
        return items;
    }

    private static string? ResolvePath(string id)
    {
        if (id.Equals("Microsoft.Windows.Explorer", StringComparison.OrdinalIgnoreCase))
            return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe");
        if (Path.IsPathFullyQualified(id) && File.Exists(id))
            return id;
        if (id.Length < 39 || id[0] != '{' || id[38] != '\\' || !Guid.TryParse(id[..38], out var folder) ||
            SHGetKnownFolderPath(ref folder, 0, IntPtr.Zero, out var path) != 0)
            return null;

        try
        {
            return Marshal.PtrToStringUni(path) is { } root ? Path.Combine(root, id[39..]) : null;
        }
        finally
        {
            Marshal.FreeCoTaskMem(path);
        }
    }

    private static string PinnedFolder => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        @"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar");

    private static Bitmap? ReadIcon(string? path)
    {
        if (path is null || SHParseDisplayName(path, IntPtr.Zero, out var item, 0, out _) != 0)
            return null;
        try
        {
            var list = SHGetFileInfo(item, 0, out var info, (uint)Marshal.SizeOf<ShellFileInfo>(), ShellSystemIconIndex | ShellPidl);
            var handle = list == IntPtr.Zero ? IntPtr.Zero : ImageList_GetIcon(list, info.IconIndex, 1);
            if (handle == IntPtr.Zero)
                return null;
            try
            {
                using var icon = Icon.FromHandle(handle);
                return icon.ToBitmap();
            }
            finally
            {
                DestroyIcon(handle);
            }
        }
        finally
        {
            Marshal.FreeCoTaskMem(item);
        }
    }

    public void Dispose()
    {
        foreach (var icon in Icons)
            icon.Image.Dispose();
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct ShellFileInfo
    {
        public IntPtr Icon;
        public int IconIndex;
        public uint Attributes;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string DisplayName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)] public string TypeName;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr FindWindow(string className, string? name);

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern int SHParseDisplayName(string name, IntPtr binding, out IntPtr item, uint flags, out uint attributes);

    [DllImport("shell32.dll", EntryPoint = "SHGetFileInfoW")]
    private static extern IntPtr SHGetFileInfo(IntPtr item, uint attributes, out ShellFileInfo info, uint size, uint flags);

    [DllImport("shell32.dll")]
    private static extern int SHGetKnownFolderPath(ref Guid folder, uint flags, IntPtr token, out IntPtr path);

    [DllImport("user32.dll")]
    private static extern bool DestroyIcon(IntPtr icon);

    [DllImport("comctl32.dll")]
    private static extern IntPtr ImageList_GetIcon(IntPtr list, int index, uint flags);
}
