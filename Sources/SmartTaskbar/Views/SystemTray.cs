using System.ComponentModel;
using System.Diagnostics;
using Windows.UI.ViewManagement;
using SmartTaskbar.Languages;

namespace SmartTaskbar
{
    internal sealed class SystemTray : ApplicationContext
    {
        private readonly Container _container = new();
        private readonly Engine _engine = new();
        private readonly ContextMenuStrip _menu;
        private readonly NotifyIcon _notifyIcon;
        private readonly Font _font = new("Segoe UI", 10.5F);
        private readonly SynchronizationContext _uiContext;
        private bool _disposed;

        public SystemTray()
        {
            var resources = new ResourceCulture();
            var about = new ToolStripMenuItem(resources.GetString(LangName.About)) { Font = _font };
            var animation = new ToolStripMenuItem(resources.GetString(LangName.Animation)) { Font = _font };
            var auto = new ToolStripMenuItem(resources.GetString(LangName.Auto)) { Font = _font };
            var dock = new ToolStripMenuItem(resources.GetString(LangName.Dock)) { Font = _font };
            var showOnExit = new ToolStripMenuItem(resources.GetString(LangName.ShowBarOnExit)) { Font = _font };
            var exit = new ToolStripMenuItem(resources.GetString(LangName.Exit)) { Font = _font };

            _menu = new ContextMenuStrip(_container) { Renderer = new Win11Renderer() };
            _menu.Items.AddRange([about, animation, new ToolStripSeparator(), dock, auto,
                new ToolStripSeparator(), showOnExit, exit]);

            _notifyIcon = new NotifyIcon(_container)
            {
                Text = Application.ProductName,
                Icon = GetIcon(),
                ContextMenuStrip = _menu,
                Visible = true
            };

            _uiContext = SynchronizationContext.Current ?? new WindowsFormsSynchronizationContext();
            Fun.UiSettings.ColorValuesChanged += OnColorChanged;

            _menu.Opening += (_, _) =>
            {
                animation.Checked = Fun.IsEnableTaskbarAnimation();
                var mode = UserSettings.AutoModeType;
                auto.Checked = mode == AutoModeType.Auto;
                dock.Checked = mode == AutoModeType.Dock;
                showOnExit.Checked = UserSettings.ShowTaskbarWhenExit;
            };
            _menu.Opened += (_, _) => _engine.MenuOpen = true;
            _menu.Closed += (_, _) => _engine.MenuOpen = false;
            _notifyIcon.MouseDoubleClick += (_, args) =>
            {
                if (args.Button != MouseButtons.Left)
                    return;

                ChangeMode(AutoModeType.None);
                Fun.SetAutoHide(!Fun.IsAutoHide());
                HideAutoHiddenBar();
            };
            about.Click += (_, _) => Process.Start(new ProcessStartInfo("https://github.com/Oliviaophia/SmartTaskbar")
                { UseShellExecute = true });
            animation.Click += (_, _) => animation.Checked = Fun.ChangeTaskbarAnimation();
            showOnExit.Click += (_, _) => UserSettings.ShowTaskbarWhenExit = !showOnExit.Checked;
            auto.Click += (_, _) => ToggleMode(AutoModeType.Auto, auto.Checked);
            dock.Click += (_, _) => ToggleMode(AutoModeType.Dock, dock.Checked);
            exit.Click += (_, _) => ExitThread();

            _engine.SetMode(UserSettings.AutoModeType);
        }

        private void ToggleMode(AutoModeType mode, bool isChecked)
        {
            ChangeMode(isChecked ? AutoModeType.None : mode);
            HideAutoHiddenBar();
        }

        private void ChangeMode(AutoModeType mode)
        {
            UserSettings.AutoModeType = mode;
            _engine.SetMode(mode);
        }

        private static Icon GetIcon() => Fun.IsLightTheme() ? IconResource.Logo_Black : IconResource.Logo_White;

        private void OnColorChanged(UISettings sender, object args)
            => _uiContext.Post(_ =>
            {
                if (!_disposed)
                    _notifyIcon.Icon = GetIcon();
            }, null);

        private static void HideAutoHiddenBar()
        {
            var shell = Fun.FindWindow(TaskbarHelper.PrimaryClass, null);
            if (shell != IntPtr.Zero && Fun.IsAutoHide())
                TaskbarHelper.Hide(shell);
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing && !_disposed)
            {
                _disposed = true;
                Fun.UiSettings.ColorValuesChanged -= OnColorChanged;
                _engine.Dispose();
                if (UserSettings.ShowTaskbarWhenExit)
                    Fun.SetAutoHide(false);
                else
                    HideAutoHiddenBar();
                _container.Dispose();
                _font.Dispose();
            }

            base.Dispose(disposing);
        }
    }
}
