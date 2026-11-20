using System.ComponentModel;
using System.Diagnostics;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using Microsoft.Win32;

namespace SmartTaskbar.Appearance;

internal readonly record struct SearchRedirectOptions(bool Enabled, string Provider);

internal static class SearchRedirectIntegration
{
    public static readonly string[] Providers = ["Google", "DuckDuckGo", "Brave", "Bing", "Ecosia", "Startpage", "Yahoo"];
    private const string Release = "https://github.com/rcmaehl/MSEdgeRedirect/releases/download/0.8.0.0/MSEdgeRedirect.exe";
    // SHA-256 from the upstream 0.8.0.0 release's checksums.sha256.
    private const string ExpectedHash = "F3C4F9DE4553E9F88B71CE189D3D1234FA560D247443D4C2D417E3B6461CD36A";
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string RunName = "SmartTaskbar.SearchRedirect";
    internal const string ResumeArgument = "--resume-search";
    private static string Folder => Path.Combine(AppContext.BaseDirectory, "OptionalFeatures", "SearchRedirect");
    private static string Helper => Path.Combine(Folder, "MSEdgeRedirect.exe");
    private static string Ini => Path.Combine(Folder, "MSEdgeRedirect", "Settings.ini");
    private static string Command => $"\"{Path.Combine(AppContext.BaseDirectory, "SmartTaskbar.Appearance.exe")}\" {ResumeArgument}";

    public static SearchRedirectOptions Read()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey);
        var enabled = string.Equals(key?.GetValue(RunName) as string, Command, StringComparison.OrdinalIgnoreCase);
        var selected = File.Exists(Ini) ? File.ReadLines(Ini)
            .FirstOrDefault(line => line.StartsWith("Search=", StringComparison.Ordinal))?[7..] : null;
        return new(enabled, Array.Find(Providers, provider => provider == selected) ?? Providers[0]);
    }

    public static Task ApplyAsync(SearchRedirectOptions options) => Task.Run(() =>
    {
        using var mutex = Acquire();
        try { Apply(options); }
        finally { mutex.ReleaseMutex(); }
    });

    internal static void Resume()
    {
        using var mutex = Acquire();
        try
        {
            if (!Read().Enabled || HasExternalInstallation() || EdgeIsDefault() || !File.Exists(Helper))
                return;
            CheckHash(File.ReadAllBytes(Helper));
            using var active = FindHelper(rejectExternal: true);
            if (active is null)
                StartHelper().Dispose();
        }
        finally { mutex.ReleaseMutex(); }
    }

    private static Mutex Acquire()
    {
        var mutex = new Mutex(false, @"Local\SmartTaskbar.SearchRedirect.Settings");
        try
        {
            if (mutex.WaitOne(TimeSpan.FromSeconds(65)))
                return mutex;
        }
        catch (AbandonedMutexException) { return mutex; }
        mutex.Dispose();
        throw new InvalidOperationException("Another search settings change is still running.");
    }

    private static void Apply(SearchRedirectOptions options)
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey, writable: true);
        var registered = key?.GetValue(RunName) as string;
        if (registered is not null && !string.Equals(registered, Command, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Another copy of SmartTaskbar manages search redirection.");
        if (!options.Enabled)
        {
            if (File.Exists(Helper) && FindHelper(rejectExternal: false) is { } active)
            {
                using (active) StopHelper(active);
            }
            if (registered is not null)
                key!.DeleteValue(RunName, false);
            return;
        }

        if (!Providers.Contains(options.Provider))
            throw new ArgumentException("Choose one of the available search providers.");
        if (HasExternalInstallation())
            throw new InvalidOperationException("Windows already has a separate MSEdgeRedirect installation. Manage it in its own settings.");
        if (EdgeIsDefault())
            throw new InvalidOperationException("Choose another Windows default browser before enabling search redirection.");
        using var existing = FindHelper(rejectExternal: true);
        EnsureHelper();
        var previous = File.Exists(Ini) ? File.ReadAllBytes(Ini) : null;
        var convert = options.Provider == "Bing" ? 0 : 1;
        var settings = $"[Settings]\r\nNoBing={convert}\r\nSearch={options.Provider}\r\nNoImgs={convert}\r\nImages={options.Provider}\r\nNoUpdates=1\r\n";
        WriteAtomic(Ini, Encoding.ASCII.GetBytes(settings));
        Process? started = null;
        try
        {
            if (existing is null)
                started = StartHelper();
            using var startup = Registry.CurrentUser.CreateSubKey(RunKey);
            startup.SetValue(RunName, Command, RegistryValueKind.String);
        }
        catch
        {
            try { if (started is not null) StopHelper(started); }
            finally
            {
                if (previous is null) File.Delete(Ini);
                else WriteAtomic(Ini, previous);
            }
            throw;
        }
        finally { started?.Dispose(); }
    }

    private static bool EdgeIsDefault()
    {
        const string associations = @"Software\Microsoft\Windows\Shell\Associations\UrlAssociations";
        foreach (var protocol in new[] { "http", "https" })
        {
            using var choice = Registry.CurrentUser.OpenSubKey($@"{associations}\{protocol}\UserChoice");
            if (choice?.GetValue("ProgId") is string app && app.StartsWith("MSEdge", StringComparison.OrdinalIgnoreCase))
                return true;
        }
        return false;
    }

    private static bool HasExternalInstallation()
    {
        foreach (var hive in new[] { RegistryHive.CurrentUser, RegistryHive.LocalMachine })
        foreach (var view in new[] { RegistryView.Registry64, RegistryView.Registry32 })
        {
            using var root = RegistryKey.OpenBaseKey(hive, view);
            using var uninstall = root.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\MSEdgeRedirect");
            if (uninstall is not null)
                return true;
        }
        return false;
    }

    private static Process? FindHelper(bool rejectExternal)
    {
        Process? result = null;
        using var current = Process.GetCurrentProcess();
        try
        {
            foreach (var process in Process.GetProcessesByName("MSEdgeRedirect"))
            {
                try
                {
                    if (process.HasExited || process.SessionId != current.SessionId)
                        continue;
                    if (string.Equals(process.MainModule?.FileName, Helper, StringComparison.OrdinalIgnoreCase))
                    {
                        result ??= process;
                    }
                    else if (rejectExternal)
                        throw new InvalidOperationException("A separate MSEdgeRedirect is running. Manage it in its own settings.");
                }
                catch (Win32Exception) when (!rejectExternal) { }
                finally { if (process != result) process.Dispose(); }
            }
            return result;
        }
        catch { result?.Dispose(); throw; }
    }

    private static Process StartHelper()
    {
        var start = new ProcessStartInfo(Helper) { UseShellExecute = false, WorkingDirectory = Folder };
        start.ArgumentList.Add("/portable");
        start.ArgumentList.Add("/hide");
        var process = Process.Start(start) ?? throw new InvalidOperationException("Could not start the search redirector.");
        if (!process.WaitForExit(300))
            return process;
        var exitCode = process.ExitCode;
        process.Dispose();
        throw new InvalidOperationException($"The search redirector exited during startup (code {exitCode}).");
    }

    private static void StopHelper(Process process)
    {
        if (process.HasExited) return;
        process.Kill();
        if (!process.WaitForExit(3000))
            throw new InvalidOperationException("The SmartTaskbar search redirector is still stopping.");
    }

    private static void EnsureHelper()
    {
        if (File.Exists(Helper))
        {
            CheckHash(File.ReadAllBytes(Helper));
            return;
        }
        using var client = new HttpClient { Timeout = TimeSpan.FromSeconds(45), MaxResponseContentBufferSize = 2_000_000 };
        var package = client.GetByteArrayAsync(Release).GetAwaiter().GetResult();
        CheckHash(package);
        WriteAtomic(Helper, package);
    }

    private static void CheckHash(byte[] package)
    {
        if (Convert.ToHexString(SHA256.HashData(package)) != ExpectedHash)
            throw new InvalidDataException("The search redirector did not match the official release checksum.");
    }

    private static void WriteAtomic(string target, byte[] contents)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(target)!);
        var temporary = target + "." + Guid.NewGuid().ToString("N");
        try
        {
            File.WriteAllBytes(temporary, contents);
            File.Move(temporary, target, overwrite: true);
        }
        finally { File.Delete(temporary); }
    }
}
