namespace SmartTaskbar.Appearance;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
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

        ApplicationConfiguration.Initialize();
        Application.Run(new AppearanceWindow(styles));
    }
}
