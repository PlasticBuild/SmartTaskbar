using System.ComponentModel;
using System.Diagnostics;

namespace SmartTaskbar.Appearance;

internal sealed class SearchRedirectWindow : Form
{
    private readonly CheckBox _enabled = new() { Text = "Open Start web links in my &default browser", AutoSize = true };
    private readonly ComboBox _provider = new() { DropDownStyle = ComboBoxStyle.DropDownList, AccessibleName = "Search provider", Dock = DockStyle.Fill };
    private readonly Label _status = new() { AutoSize = true, Dock = DockStyle.Fill, AccessibleRole = AccessibleRole.StatusBar };
    private readonly Button _save = new() { Text = "&Save changes", AutoSize = true, Enabled = false };
    private readonly Button _close = new() { Text = "Close", DialogResult = DialogResult.Cancel, AutoSize = true };
    private SearchRedirectOptions? _current;
    private bool _saving;

    public SearchRedirectWindow()
    {
        var theme = AppearanceTheme.Read();
        Text = "Start web searches";
        StartPosition = FormStartPosition.CenterParent;
        AutoScaleDimensions = new SizeF(96, 96);
        AutoScaleMode = AutoScaleMode.Dpi;
        Font = SystemFonts.MessageBoxFont;
        ShowInTaskbar = false;
        MinimizeBox = MaximizeBox = false;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        BackColor = theme.Window;
        ForeColor = theme.Text;
        AutoSize = true;
        AutoSizeMode = AutoSizeMode.GrowAndShrink;

        var content = new TableLayoutPanel
        {
            ColumnCount = 1, Dock = DockStyle.Fill, AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink, Padding = new Padding(20)
        };
        content.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 408));
        Controls.Add(content);

        void Add(Control control, int gap = 7)
        {
            control.Margin = new Padding(0, 0, 0, gap);
            content.Controls.Add(control, 0, content.RowCount++);
        }

        Label Explanation(string text) => new()
        {
            Text = text, AutoSize = true, Dock = DockStyle.Fill, ForeColor = theme.Muted,
            MaximumSize = new Size(408, 0)
        };

        Add(new Label { Text = "Start web searches", Font = new Font(Font.FontFamily, 15, FontStyle.Bold), AutoSize = true });
        Add(_enabled);
        Add(Explanation("Enabling this option downloads MSEdgeRedirect (about 1.1 MB) and runs it now and at sign-in. It also handles Edge links opened elsewhere in Windows. Its portable mode can interrupt other apps launched by the Windows shell."), 16);
        Add(new Label { Text = "Search &provider", AutoSize = true }, 3);
        _provider.Items.AddRange(SearchRedirectIntegration.Providers);
        Add(_provider);
        Add(Explanation("Bing web and image search links open with this provider. Previews inside Start use the Windows search provider."), 14);
        var defaults = new LinkLabel
        {
            Text = "Choose my Windows default browser", AutoSize = true,
            LinkColor = theme.Accent, ActiveLinkColor = theme.Accent, VisitedLinkColor = theme.Accent
        };
        defaults.LinkClicked += (_, _) =>
        {
            try { Process.Start(new ProcessStartInfo("ms-settings:defaultapps") { UseShellExecute = true }); }
            catch (Win32Exception error) { _status.Text = error.Message; }
        };
        Add(defaults);
        _status.MaximumSize = new Size(408, 0);
        _status.ForeColor = theme.Muted;
        Add(_status);

        var actions = new FlowLayoutPanel
        {
            FlowDirection = FlowDirection.RightToLeft, Dock = DockStyle.Fill, AutoSize = true, WrapContents = false
        };
        actions.Controls.AddRange([_save, _close]);
        Add(actions, 0);
        AcceptButton = _save;
        CancelButton = _close;
        _enabled.CheckedChanged += (_, _) => UpdateButtons();
        _provider.SelectedIndexChanged += (_, _) => UpdateButtons();
        _save.Click += async (_, _) => await SaveAsync();
        FormClosing += (_, e) => e.Cancel = _saving && e.CloseReason == CloseReason.UserClosing;
        Load += (_, _) =>
        {
            var width = Math.Min(LogicalToDeviceUnits(408), Screen.FromControl(this).WorkingArea.Width - LogicalToDeviceUnits(56));
            content.ColumnStyles[0].Width = width;
            foreach (var label in content.Controls.OfType<Label>())
                if (label.MaximumSize.Width > 0)
                    label.MaximumSize = new Size(width, 0);
        };
        Shown += (_, _) =>
        {
            try
            {
                var current = SearchRedirectIntegration.Read();
                _current = current;
                _enabled.Checked = current.Enabled;
                _provider.SelectedItem = current.Provider;
                if (_provider.SelectedIndex < 0 && _provider.Items.Count > 0)
                    _provider.SelectedIndex = 0;
            }
            catch (Exception error) { _status.Text = error.Message; }
            UpdateButtons();
        };
        HandleCreated += (_, _) => theme.ApplyTitleBar(Handle);
    }

    private void UpdateButtons()
    {
        _enabled.Enabled = !_saving && _current is not null;
        _provider.Enabled = _enabled.Enabled && _enabled.Checked;
        _save.Enabled = _enabled.Enabled && _provider.SelectedItem is string provider &&
            new SearchRedirectOptions(_enabled.Checked, provider) != _current;
        _close.Enabled = !_saving;
    }

    private async Task SaveAsync()
    {
        if (_provider.SelectedItem is not string provider)
            return;
        _saving = true;
        _status.Text = _enabled.Checked ? "Setting up web search…" : "Turning off web search redirection…";
        UpdateButtons();
        try
        {
            await SearchRedirectIntegration.ApplyAsync(new SearchRedirectOptions(_enabled.Checked, provider));
            _saving = false;
            DialogResult = DialogResult.OK;
        }
        catch (Exception error) { _status.Text = error.Message; }
        finally
        {
            _saving = false;
            UpdateButtons();
        }
    }
}
