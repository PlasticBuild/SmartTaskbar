namespace SmartTaskbar.Appearance;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        if (args is [IslandMenuWidth.Argument, ..])
        {
            if (args.Length is < 2 or > 4 || !int.TryParse(args[1], out var width))
                return;
            int? shift = null;
            if (args.Length >= 3 && args[2] != "native")
            {
                if (!int.TryParse(args[2], out var value) || value > 0)
                    return;
                shift = value;
            }
            var searchInset = 0;
            if (args.Length == 4 && (!int.TryParse(args[3], out searchInset) || searchInset < 0))
                return;
            if (args.Length < 4)
                searchInset = PrimaryIslandWidth.Read().SearchInset;
            try
            {
                var synced = new TaskbarStyles(AppContext.BaseDirectory)
                    .SyncStartWidthAsync(width, args.Length >= 3, shift, searchInset).GetAwaiter().GetResult();
                if (!synced) Environment.ExitCode = 2;
            }
            catch (Exception error) when (TaskbarStyles.IsServiceError(error))
            {
                System.Diagnostics.Trace.WriteLine(error.Message);
                Environment.ExitCode = 1;
            }
            return;
        }
        if (args is [SearchRedirectIntegration.ResumeArgument])
        {
            try { SearchRedirectIntegration.Resume(); }
            catch (Exception error) when (TaskbarStyles.IsServiceError(error) || error is System.Security.SecurityException)
            {
                System.Diagnostics.Trace.WriteLine(error.Message);
            }
            return;
        }

        var styles = new TaskbarStyles(AppContext.BaseDirectory);
        if (args is ["--resume"])
        {
            try { styles.ResumeAsync().GetAwaiter().GetResult(); }
            catch (Exception error) when (TaskbarStyles.IsServiceError(error))
            {
                System.Diagnostics.Trace.WriteLine(error.Message);
            }
            return;
        }

        using var instance = new Mutex(true, @"Local\SmartTaskbar.Appearance", out var created);
        if (!created)
            return;

#pragma warning disable WFO5000 // Opt in to Windows theme support for native WinForms controls.
        Application.SetColorMode(SystemColorMode.System);
#pragma warning restore WFO5000
        ApplicationConfiguration.Initialize();
        Application.Run(new AppearanceWindow(styles));
    }
}
