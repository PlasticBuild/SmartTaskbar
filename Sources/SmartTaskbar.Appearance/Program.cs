namespace SmartTaskbar.Appearance;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        if (args is [IslandMenuWidth.Argument, var rawWidth] && int.TryParse(rawWidth, out var width))
        {
            try { new TaskbarStyles(AppContext.BaseDirectory).SyncStartWidthAsync(width).GetAwaiter().GetResult(); }
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

        using var instance = new Mutex(true, @"Local\SmartTaskbar.Appearance", out var created);
        if (!created)
            return;

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

#pragma warning disable WFO5000 // Opt in to Windows theme support for native WinForms controls.
        Application.SetColorMode(SystemColorMode.System);
#pragma warning restore WFO5000
        ApplicationConfiguration.Initialize();
        Application.Run(new AppearanceWindow(styles));
    }
}
