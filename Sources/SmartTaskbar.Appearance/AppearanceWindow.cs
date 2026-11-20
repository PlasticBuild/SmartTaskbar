using Microsoft.Win32;
using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar.Appearance;

internal sealed class AppearanceWindow : Form
{
    private readonly TaskbarStyles _styles;
    private readonly TaskbarSandbox _preview = new() { Dock = DockStyle.Fill, Margin = Padding.Empty };
    private readonly Timer _previewRefresh = new() { Interval = 4000 };
    private readonly RadioButton _fullWidth = LayoutChoice("Full &width");
    private readonly RadioButton _island = LayoutChoice("&Floating island");
    private readonly CheckBox _fullscreen = new()
    {
        Text = "Simulate &fullscreen", Appearance = System.Windows.Forms.Appearance.Button,
        AutoSize = true, MinimumSize = new Size(138, 32), TextAlign = ContentAlignment.MiddleCenter
    };
    private readonly Label _previewHint = new() { AutoSize = true, Dock = DockStyle.Fill, Margin = new Padding(0, 9, 0, 0) };
    private readonly Button _apply = new() { Text = "&Apply changes", AutoSize = true, Enabled = false };
    private readonly LinkLabel _restore = new() { Text = "&Restore Windows look", AutoSize = true, Enabled = false };
    private readonly LinkLabel _webSearch = new() { Text = "Configure Start &web searches…", AutoSize = true };
    private readonly CheckBox _animateMenus = new() { Text = "&Animate menus", AccessibleName = "Animate menus", AutoSize = true };
    private readonly Button _menuPreview = new() { Text = "Preview &menu", AccessibleName = "Preview menu", AutoSize = true };
    private readonly Label _status = new() { AutoSize = true, AccessibleName = "Layout status" };
    private readonly TrackBar _radius = Slider("Corner radius in pixels", AppearanceOptions.MaxRadius, 1);
    private readonly TrackBar _transparency = Slider("Transparency percentage", AppearanceOptions.MaxTransparency, 5);
    private readonly TrackBar _hoverLift = Slider("Hover lift in pixels", AppearanceOptions.MaxHoverLift, 1);
    private readonly TrackBar _clickDepth = Slider("Click depth in pixels", AppearanceOptions.MaxClickDepth, 1);
    private readonly ComboBox _activationZone = new()
    {
        DropDownStyle = ComboBoxStyle.DropDownList, DrawMode = DrawMode.OwnerDrawFixed,
        AccessibleName = "Activation zone", Dock = DockStyle.Fill, Margin = Padding.Empty
    };
    private readonly ComboBox _alignment = new()
    {
        DropDownStyle = ComboBoxStyle.DropDownList, DrawMode = DrawMode.OwnerDrawFixed,
        AccessibleName = "Island position", Width = 84, Margin = new Padding(6, 0, 0, 0)
    };
    private readonly List<Control> _cards = [];
    private readonly List<Label> _secondaryText = [];
    private AppearanceTheme _theme;
    private AppearanceOptions _current = new(TaskbarLayout.Windows);
    private RevealZone _currentZone;
    private bool _currentMenuAnimations;
    private bool _preferencesLoaded;
    private AppearanceOptions _loadedSelection;
    private bool _readingIcons;
    private bool _running;
    private bool _presetCurrent;
    private bool _busy;
    private bool _available;

    public AppearanceWindow(TaskbarStyles styles)
    {
        _styles = styles;
        Text = "SmartTaskbar Appearance";
        AccessibleName = Text;
        AutoScaleDimensions = new SizeF(96, 96);
        AutoScaleMode = AutoScaleMode.Dpi;
        StartPosition = FormStartPosition.CenterScreen;
        Font = SystemFonts.MessageBoxFont;
        Icon = Icon.ExtractAssociatedIcon(Environment.ProcessPath!);
        ClientSize = new Size(1000, 610);

        _fullWidth.Checked = true;
        _radius.Value = _current.Radius;
        _transparency.Value = _current.Transparency;
        _hoverLift.Value = _current.HoverLift;
        _clickDepth.Value = _current.ClickDepth;
        _activationZone.Items.AddRange(["Entire edge", "Center half", "Taskbar width"]);
        _activationZone.SelectedIndex = 0;
        _activationZone.DrawItem += DrawChoice;
        _alignment.Items.AddRange([IslandAlignment.Left, IslandAlignment.Center, IslandAlignment.Right]);
        _alignment.SelectedItem = IslandAlignment.Center;
        _alignment.DrawItem += DrawChoice;

        var root = Table(1);
        root.Padding = new Padding(24, 19, 24, 18);
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        Controls.Add(root);

        root.Controls.Add(new Label
        {
            Text = "Taskbar appearance", AutoSize = true, Font = new Font(Font.FontFamily, 21, FontStyle.Bold),
            Margin = new Padding(0, 0, 0, 16)
        }, 0, 0);

        var content = Table(2);
        content.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 65));
        content.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 35));
        content.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        content.Controls.Add(CreatePreview(), 0, 0);
        content.Controls.Add(CreateSettings(), 1, 0);
        root.Controls.Add(content, 0, 1);
        root.Controls.Add(CreateFooter(), 0, 2);

        Shown += async (_, _) =>
        {
            _previewRefresh.Start();
            await Task.WhenAll(RunAsync(null), RefreshIcons());
        };
        _previewRefresh.Tick += async (_, _) => await RefreshIcons();
        _island.CheckedChanged += (_, _) => RefreshSelection();
        _fullWidth.CheckedChanged += (_, _) => RefreshSelection();
        _radius.ValueChanged += (_, _) => RefreshSelection();
        _transparency.ValueChanged += (_, _) => RefreshSelection();
        _hoverLift.ValueChanged += (_, _) => RefreshSelection();
        _clickDepth.ValueChanged += (_, _) => RefreshSelection();
        _activationZone.SelectedIndexChanged += (_, _) => RefreshSelection();
        _alignment.SelectedIndexChanged += (_, _) => RefreshSelection();
        _animateMenus.CheckedChanged += (_, _) => RefreshSelection();
        _menuPreview.Click += (_, _) => _preview.ToggleMenu();
        _fullscreen.CheckedChanged += (_, _) =>
        {
            _preview.SetFullscreen(_fullscreen.Checked);
            RefreshSelection();
        };
        _apply.Click += async (_, _) => await RunAsync(Selection);
        _restore.LinkClicked += async (_, _) => await RunAsync(new AppearanceOptions(TaskbarLayout.Windows));
        _webSearch.LinkClicked += (_, _) =>
        {
            using var dialog = new SearchRedirectWindow();
            dialog.ShowDialog(this);
        };
        FormClosing += (_, args) => args.Cancel = _busy && args.CloseReason == CloseReason.UserClosing;
        SystemEvents.UserPreferenceChanged += OnUserPreferenceChanged;
        RefreshTheme();
        RefreshSelection();
    }

    protected override void OnLoad(EventArgs args)
    {
        base.OnLoad(args);
        var display = Screen.FromHandle(Handle).WorkingArea;
        var frame = SizeFromClientSize(Size.Empty);
        var width = Math.Min(LogicalToDeviceUnits(1000), display.Width - frame.Width);
        var height = Math.Min(LogicalToDeviceUnits(610), display.Height - frame.Height);
        ClientSize = new Size(width, height);
        MinimumSize = SizeFromClientSize(new Size(Math.Min(width, LogicalToDeviceUnits(730)),
            Math.Min(height, LogicalToDeviceUnits(490))));
        _theme.ApplyTitleBar(Handle);
        CenterToScreen();
    }

    private TableLayoutPanel CreatePreview()
    {
        var card = Card();
        card.AutoSize = false;
        card.Margin = new Padding(0, 0, 16, 0);
        card.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        card.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        card.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        var header = Table(2);
        header.AutoSize = true;
        header.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        header.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        header.Controls.Add(Heading("Live preview"), 0, 0);
        var layouts = new FlowLayoutPanel
        {
            AutoSize = true, WrapContents = false, Margin = new Padding(0, 4, 0, 10), Dock = DockStyle.Fill
        };
        layouts.Controls.AddRange([_island, _fullWidth]);
        header.Controls.Add(layouts, 0, 1);
        _fullscreen.Margin = new Padding(5, 4, 0, 10);
        _fullscreen.Anchor = AnchorStyles.Right;
        header.Controls.Add(_fullscreen, 1, 0);
        var position = new FlowLayoutPanel
        {
            AutoSize = true, WrapContents = false, Anchor = AnchorStyles.Right, Margin = new Padding(0, 4, 0, 10)
        };
        var label = new Label { Text = "&Position", AutoSize = true, Anchor = AnchorStyles.Left, Margin = Padding.Empty };
        position.Controls.AddRange([label, _alignment]);
        header.Controls.Add(position, 1, 1);

        card.Controls.Add(header, 0, 0);
        card.Controls.Add(_preview, 0, 1);
        _secondaryText.Add(_previewHint);
        card.Controls.Add(_previewHint, 0, 2);
        return card;
    }

    private Panel CreateSettings()
    {
        var rail = new Panel { Dock = DockStyle.Fill, AutoScroll = true, Margin = Padding.Empty };
        var cards = Table(1);
        cards.Dock = DockStyle.Top;
        cards.AutoSize = true;
        rail.Controls.Add(cards);

        var glass = Card();
        glass.Controls.Add(Heading("Glass"), 0, 0);
        glass.Controls.Add(SliderRow("Corner radius", _radius, " px"), 0, 1);
        glass.Controls.Add(SliderRow("Transparency", _transparency, "%"), 0, 2);
        cards.Controls.Add(glass, 0, 0);

        var motion = Card();
        motion.Controls.Add(Heading("Motion"), 0, 0);
        motion.Controls.Add(SliderRow("Hover lift", _hoverLift, " px"), 0, 1);
        motion.Controls.Add(SliderRow("Click depth", _clickDepth, " px"), 0, 2);
        var menuRow = new FlowLayoutPanel { AutoSize = true, Dock = DockStyle.Fill, Margin = Padding.Empty };
        _animateMenus.Margin = new Padding(0, 5, 9, 0);
        menuRow.Controls.AddRange([_animateMenus, _menuPreview]);
        motion.Controls.Add(menuRow, 0, 3);
        cards.Controls.Add(motion, 0, 1);

        var behavior = Card();
        behavior.Controls.Add(Heading("Intelligent auto-hide"), 0, 0);
        var zoneLabel = new Label { Text = "Activation &zone", AutoSize = true, Margin = new Padding(0, 1, 0, 5) };
        behavior.Controls.Add(zoneLabel, 0, 1);
        behavior.Controls.Add(_activationZone, 0, 2);
        cards.Controls.Add(behavior, 0, 2);
        _webSearch.Margin = new Padding(0, 3, 0, 5);
        cards.Controls.Add(_webSearch, 0, 3);
        return rail;
    }

    private TableLayoutPanel SliderRow(string label, TrackBar slider, string unit)
    {
        var row = Table(2);
        row.AutoSize = true;
        row.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        row.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        row.Controls.Add(new Label { Text = label, AutoSize = true, Margin = Padding.Empty }, 0, 0);
        var value = new Label { AutoSize = true, Anchor = AnchorStyles.Right, Margin = Padding.Empty };
        value.Text = slider.Value + unit;
        slider.ValueChanged += (_, _) => value.Text = slider.Value + unit;
        _secondaryText.Add(value);
        row.Controls.Add(value, 1, 0);
        row.Controls.Add(slider, 0, 1);
        row.SetColumnSpan(slider, 2);
        return row;
    }

    private TableLayoutPanel CreateFooter()
    {
        var footer = Table(3);
        footer.AutoSize = true;
        footer.Margin = new Padding(0, 14, 0, 0);
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        _status.Text = "Reading the current layout…";
        _status.Margin = new Padding(0, 0, 8, 0);
        _status.Anchor = AnchorStyles.Left;
        _secondaryText.Add(_status);
        footer.Controls.Add(_status, 0, 0);
        _restore.Anchor = AnchorStyles.Right;
        _restore.Margin = new Padding(0, 0, 15, 0);
        footer.Controls.Add(_restore, 1, 0);
        _apply.MinimumSize = new Size(132, 36);
        _apply.Margin = Padding.Empty;
        footer.Controls.Add(_apply, 2, 0);
        return footer;
    }

    private TableLayoutPanel Card()
    {
        var card = Table(1);
        card.AutoSize = true;
        card.Padding = new Padding(14, 11, 14, 10);
        card.Margin = new Padding(0, 0, 0, 10);
        _cards.Add(card);
        return card;
    }

    private static TableLayoutPanel Table(int columns) => new()
    {
        Dock = DockStyle.Fill, ColumnCount = columns, Margin = Padding.Empty
    };

    private Label Heading(string heading) => new()
    {
        Text = heading, Font = new Font(Font.FontFamily, 11, FontStyle.Bold),
        AutoSize = true, Margin = new Padding(0, 0, 0, 8)
    };

    private static RadioButton LayoutChoice(string text) => new()
    {
        Text = text, Appearance = System.Windows.Forms.Appearance.Button,
        AutoSize = true, TextAlign = ContentAlignment.MiddleCenter,
        MinimumSize = new Size(108, 32), Margin = new Padding(0, 0, 5, 0)
    };

    private static TrackBar Slider(string name, int maximum, int step) => new()
    {
        AccessibleName = name, Maximum = maximum, SmallChange = step, LargeChange = step,
        TickStyle = TickStyle.None, AutoSize = false, Height = 23, Dock = DockStyle.Fill, Margin = new Padding(0, 1, 0, 5)
    };

    private void OnUserPreferenceChanged(object sender, UserPreferenceChangedEventArgs args)
    {
        if (IsHandleCreated && !IsDisposed)
            BeginInvoke((Action)RefreshTheme);
    }

    private void RefreshTheme()
    {
        _theme = AppearanceTheme.Read();
        BackColor = _theme.Window;
        ForeColor = _theme.Text;
        if (IsHandleCreated)
            _theme.ApplyTitleBar(Handle);
        foreach (var card in _cards)
            card.BackColor = _theme.Card;
        foreach (var label in _secondaryText)
            label.ForeColor = _theme.Muted;
        foreach (var slider in new[] { _radius, _transparency, _hoverLift, _clickDepth })
            slider.BackColor = _theme.Card;
        _alignment.BackColor = _activationZone.BackColor = _theme.Input;
        _alignment.ForeColor = _activationZone.ForeColor = _theme.Text;
        _restore.LinkColor = _restore.ActiveLinkColor = _restore.VisitedLinkColor = _theme.Accent;
        _restore.DisabledLinkColor = _theme.Muted;
        _webSearch.LinkColor = _webSearch.ActiveLinkColor = _webSearch.VisitedLinkColor = _theme.Accent;
        _preview.SetTheme(_theme);
        RefreshButtonColors();
    }

    private void RefreshButtonColors()
    {
        StyleButton(_island, _island.Checked, false);
        StyleButton(_fullWidth, _fullWidth.Checked, false);
        StyleButton(_fullscreen, _fullscreen.Checked, false);
        StyleButton(_menuPreview, false, false);
        StyleButton(_apply, _apply.Enabled, _apply.Enabled);
    }

    private void StyleButton(ButtonBase button, bool selected, bool primary)
    {
        button.FlatStyle = FlatStyle.Flat;
        button.UseVisualStyleBackColor = false;
        button.BackColor = primary ? _theme.Accent : _theme.Input;
        button.ForeColor = primary ? _theme.AccentText : selected ? _theme.Accent : _theme.Text;
        button.FlatAppearance.BorderColor = selected ? _theme.Accent : _theme.Line;
        button.FlatAppearance.MouseOverBackColor = primary ? _theme.Accent : _theme.Card;
        button.FlatAppearance.MouseDownBackColor = primary ? _theme.Accent : _theme.Input;
        button.FlatAppearance.CheckedBackColor = _theme.Input;
    }

    private void DrawChoice(object? sender, DrawItemEventArgs args)
    {
        var selected = (args.State & DrawItemState.Selected) != 0 &&
            (args.State & DrawItemState.ComboBoxEdit) == 0;
        using var ink = new SolidBrush(selected ? _theme.Accent : _theme.Input);
        args.Graphics.FillRectangle(ink, args.Bounds);
        if (args.Index >= 0 && sender is ComboBox choice)
        {
            var textBounds = Rectangle.Inflate(args.Bounds, -4, 0);
            TextRenderer.DrawText(args.Graphics, choice.GetItemText(choice.Items[args.Index]), Font, textBounds,
                !choice.Enabled ? _theme.Muted : selected ? _theme.AccentText : _theme.Text, TextFormatFlags.Left | TextFormatFlags.VerticalCenter);
        }
        args.DrawFocusRectangle();
    }

    private AppearanceOptions Selection => new(_island.Checked ? TaskbarLayout.Island : TaskbarLayout.FullWidth,
        _radius.Value, _transparency.Value, _hoverLift.Value, _clickDepth.Value)
    {
        Alignment = _island.Checked && _alignment.SelectedItem is IslandAlignment alignment ? alignment : IslandAlignment.Center
    };

    private RevealZone SelectedZone => (RevealZone)_activationZone.SelectedIndex;

    private async Task RefreshIcons()
    {
        if (_readingIcons)
            return;
        _readingIcons = true;
        var signature = _preview.IconSignature;
        try
        {
            var icons = await Task.Run(() => TaskbarPreviewIcons.Read(signature));
            if (icons is not null)
            {
                if (IsDisposed || _preview.IsDisposed)
                    icons.Dispose();
                else
                    _preview.SetIcons(icons);
            }
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or ArgumentException)
        {
            // Keep the last snapshot if Explorer or a shortcut is temporarily unavailable.
        }
        finally
        {
            _readingIcons = false;
        }
    }

    private async Task RunAsync(AppearanceOptions? layout)
    {
        _busy = true;
        RefreshSelection();
        try
        {
            if (layout is null)
            {
                _currentZone = RevealZoneSettings.Read();
                _activationZone.SelectedIndex = (int)_currentZone;
                _currentMenuAnimations = MenuAnimationSettings.Read();
                _animateMenus.Checked = _currentMenuAnimations;
                _loadedSelection = Selection;
                _preferencesLoaded = true;
            }
            if (layout is { } selection)
            {
                if (selection.Layout != TaskbarLayout.Windows && selection == _loadedSelection && PreferencesChanged)
                {
                    SavePreferences();
                    if (!_available) _status.Text = "Behavior settings saved.";
                    return;
                }
                _status.Text = selection.Layout == TaskbarLayout.Windows ? "Restoring Windows appearance…" : "Applying the layout…";
                await _styles.ApplyAsync(selection);
                if (selection.Layout != TaskbarLayout.Windows)
                    SavePreferences();
            }

            (_current, _running, _presetCurrent) = await _styles.ReadAsync();
            _currentZone = RevealZoneSettings.Read();
            if (layout is null)
            {
                _fullWidth.Checked = _current.Layout != TaskbarLayout.Island;
                _island.Checked = _current.Layout == TaskbarLayout.Island;
                _radius.Value = _current.Radius;
                _transparency.Value = _current.Transparency;
                _hoverLift.Value = _current.HoverLift;
                _clickDepth.Value = _current.ClickDepth;
                _alignment.SelectedItem = _current.Alignment;
                _activationZone.SelectedIndex = (int)_currentZone;
            }
            _loadedSelection = Selection;
            _available = true;
            _status.Text = _current.Layout switch
            {
                TaskbarLayout.FullWidth => _running ? "Full width is active." : "Full width is saved. Apply to resume it.",
                TaskbarLayout.Island => _running ? "Floating island is active." : "Floating island is saved. Apply to resume it.",
                TaskbarLayout.Custom => "A custom appearance is configured.",
                _ => "Windows appearance is active."
            };
        }
        catch (Exception error) when (TaskbarStyles.IsServiceError(error))
        {
            _status.Text = error.Message;
        }
        finally
        {
            _busy = false;
            RefreshSelection();
        }
    }

    private bool PreferencesChanged => _currentMenuAnimations != _animateMenus.Checked || _currentZone != SelectedZone;

    private void SavePreferences()
    {
        if (_currentZone != SelectedZone)
        {
            RevealZoneSettings.Save(SelectedZone);
            _currentZone = SelectedZone;
        }
        if (_currentMenuAnimations != _animateMenus.Checked)
        {
            MenuAnimationSettings.Save(_animateMenus.Checked);
            _currentMenuAnimations = _animateMenus.Checked;
        }
    }

    private void RefreshSelection()
    {
        _island.Enabled = _fullWidth.Enabled = !_busy;
        _radius.Enabled = _transparency.Enabled = _hoverLift.Enabled = _clickDepth.Enabled = !_busy;
        _activationZone.Enabled = !_busy;
        _alignment.Enabled = !_busy && _island.Checked;
        _animateMenus.Enabled = !_busy;
        _apply.Enabled = !_busy && ((_available && (_current != Selection || !_running || !_presetCurrent)) ||
            (_preferencesLoaded && PreferencesChanged && (_available || Selection == _loadedSelection)));
        _restore.Enabled = _available && !_busy && _current.Layout != TaskbarLayout.Windows;
        _preview.SetLayout(Selection, SelectedZone);
        _preview.SetMenuAnimations(_animateMenus.Checked);
        _previewHint.Text = _fullscreen.Checked ? "Move to the highlighted edge to reveal the bar." : "Hover or click an app icon to try its motion.";
        RefreshButtonColors();
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            SystemEvents.UserPreferenceChanged -= OnUserPreferenceChanged;
            _previewRefresh.Dispose();
        }
        base.Dispose(disposing);
    }
}
