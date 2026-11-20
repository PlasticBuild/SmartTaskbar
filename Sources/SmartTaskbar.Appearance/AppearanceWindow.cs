using System.Drawing.Drawing2D;

namespace SmartTaskbar.Appearance;

internal sealed class AppearanceWindow : Form
{
    private readonly TaskbarStyles _styles;
    private readonly Color _accent = SystemInformation.HighContrast ? SystemColors.Highlight : Color.FromArgb(28, 83, 150);
    private readonly Color _text = SystemInformation.HighContrast ? SystemColors.WindowText : Color.FromArgb(27, 37, 55);
    private readonly Color _muted = SystemInformation.HighContrast ? SystemColors.WindowText : Color.FromArgb(74, 88, 108);
    private readonly RadioButton _fullWidth = new() { Text = "&Full width", AutoSize = true, Checked = true };
    private readonly RadioButton _island = new() { Text = "&Floating island", AutoSize = true };
    private readonly Button _apply = new() { Text = "&Apply", AutoSize = true, Enabled = false };
    private readonly LinkLabel _restore = new() { Text = "&Restore Windows look", AutoSize = true, Enabled = false };
    private readonly Label _status = new() { AutoSize = true, AccessibleName = "Layout status" };
    private readonly NumericUpDown _radius = new() { Minimum = 0, Maximum = AppearanceOptions.MaxRadius, Value = 8, AccessibleName = "Corner radius in pixels" };
    private readonly NumericUpDown _transparency = new() { Minimum = 0, Maximum = AppearanceOptions.MaxTransparency, Value = 40, Increment = 5, AccessibleName = "Transparency percentage" };
    private readonly NumericUpDown _hoverLift = new() { Minimum = 0, Maximum = AppearanceOptions.MaxHoverLift, Value = AppearanceOptions.DefaultHoverLift, AccessibleName = "Hover lift in pixels" };
    private readonly NumericUpDown _clickDepth = new() { Minimum = 0, Maximum = AppearanceOptions.MaxClickDepth, Value = AppearanceOptions.DefaultClickDepth, AccessibleName = "Click depth in pixels" };
    private AppearanceOptions _current = new(TaskbarLayout.Windows);
    private bool _running;
    private bool _savedWithMotion;
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
        BackColor = SystemInformation.HighContrast ? SystemColors.Window : Color.FromArgb(245, 247, 252);
        ForeColor = _text;
        ClientSize = new Size(686, 486);
        MaximizeBox = false;

        var root = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            Padding = new Padding(24, 18, 24, 20),
            ColumnCount = 1,
            RowCount = 6
        };
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        Controls.Add(root);

        var title = new Label
        {
            AutoSize = true,
            Font = new Font(Font.FontFamily, 19, FontStyle.Bold),
            Text = "Taskbar appearance",
            Margin = new Padding(0, 0, 0, 16)
        };
        root.Controls.Add(title, 0, 0);
        root.Controls.Add(CreateOptions(), 0, 1);
        root.Controls.Add(CreateSettingsRow("Corner &radius (px)", _radius, "&Transparency (%)", _transparency, 16), 0, 2);
        root.Controls.Add(CreateSettingsRow("&Hover lift (px)", _hoverLift, "C&lick depth (px)", _clickDepth, 8), 0, 3);

        _status.ForeColor = _muted;
        _status.Margin = new Padding(0, 12, 0, 10);
        _status.MaximumSize = new Size(620, 0);
        _status.Text = "Reading the current layout…";
        root.Controls.Add(_status, 0, 4);

        var footer = new TableLayoutPanel { AutoSize = true, Dock = DockStyle.Fill, ColumnCount = 2, Margin = Padding.Empty };
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
        footer.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        _restore.Anchor = AnchorStyles.Left;
        _restore.LinkColor = _accent;
        _restore.Margin = Padding.Empty;
        footer.Controls.Add(_restore, 0, 0);
        _apply.MinimumSize = new Size(112, 34);
        _apply.Margin = Padding.Empty;
        _apply.FlatStyle = FlatStyle.System;
        footer.Controls.Add(_apply, 1, 0);
        root.Controls.Add(footer, 0, 5);

        AcceptButton = _apply;
        Shown += async (_, _) => await RunAsync(null);
        _fullWidth.CheckedChanged += (_, _) => RefreshButtons();
        _island.CheckedChanged += (_, _) => RefreshButtons();
        _radius.ValueChanged += (_, _) => RefreshButtons();
        _transparency.ValueChanged += (_, _) => RefreshButtons();
        _hoverLift.ValueChanged += (_, _) => RefreshButtons();
        _clickDepth.ValueChanged += (_, _) => RefreshButtons();
        _apply.Click += async (_, _) => await RunAsync(Selection);
        _restore.LinkClicked += async (_, _) => await RunAsync(new AppearanceOptions(TaskbarLayout.Windows));
        FormClosing += (_, args) => args.Cancel = _busy && args.CloseReason == CloseReason.UserClosing;
    }

    protected override void OnLoad(EventArgs args)
    {
        var display = Screen.FromHandle(Handle).WorkingArea;
        var available = SizeFromClientSize(Size.Empty);
        var width = Math.Min(LogicalToDeviceUnits(686), display.Width - available.Width);
        var height = Math.Min(LogicalToDeviceUnits(486), display.Height - available.Height);
        ClientSize = new Size(width, height);
        MinimumSize = SizeFromClientSize(new Size(Math.Min(width, LogicalToDeviceUnits(590)),
            Math.Min(height, LogicalToDeviceUnits(430))));
        CenterToScreen();
        base.OnLoad(args);
    }

    private static FlowLayoutPanel CreateSettingsRow(string leftLabel, NumericUpDown left,
        string rightLabel, NumericUpDown right, int top)
    {
        var settings = new FlowLayoutPanel { Dock = DockStyle.Fill, AutoSize = true, Margin = new Padding(0, top, 0, 0) };
        AddField(leftLabel, left, 18);
        AddField(rightLabel, right, 0);
        return settings;

        void AddField(string label, NumericUpDown field, int spaceAfter)
        {
            settings.Controls.Add(new Label { Text = label, Width = 122, Margin = new Padding(0, 4, 6, 0) });
            field.Width = 65;
            field.Margin = new Padding(0, 0, spaceAfter, 0);
            settings.Controls.Add(field);
        }
    }

    private TableLayoutPanel CreateOptions()
    {
        var options = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 3, Margin = Padding.Empty };
        options.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50));
        options.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 50));
        options.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
        options.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        options.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        AddOption(_fullWidth, 0, "Glass across the screen with the controls in their usual Windows positions.");
        AddOption(_island, 1, "Start, apps and the system tray grouped together on a centered glass bar.");
        return options;

        void AddOption(RadioButton button, int column, string description)
        {
            var gap = column == 0 ? new Padding(0, 0, 8, 0) : new Padding(8, 0, 0, 0);
            var preview = new LayoutPreview(column == 1, _accent)
            {
                Dock = DockStyle.Fill,
                Margin = gap,
                AccessibleRole = AccessibleRole.Graphic,
                AccessibleName = column == 1 ? "Centered island layout preview" : "Full-width layout preview"
            };
            preview.Selected = button.Checked;
            button.CheckedChanged += (_, _) => preview.Selected = button.Checked;
            _radius.ValueChanged += (_, _) => preview.SetGlass((int)_radius.Value, (int)_transparency.Value);
            _transparency.ValueChanged += (_, _) => preview.SetGlass((int)_radius.Value, (int)_transparency.Value);
            _hoverLift.ValueChanged += (_, _) => preview.SetMotion((int)_hoverLift.Value, (int)_clickDepth.Value);
            _clickDepth.ValueChanged += (_, _) => preview.SetMotion((int)_hoverLift.Value, (int)_clickDepth.Value);
            preview.Click += (_, _) =>
            {
                if (button.Enabled)
                {
                    button.Checked = true;
                    button.Focus();
                }
            };
            options.Controls.Add(preview, column, 0);

            button.Font = new Font(Font.FontFamily, 11, FontStyle.Bold);
            button.Margin = new Padding(gap.Left, 10, gap.Right, 4);
            options.Controls.Add(button, column, 1);
            options.Controls.Add(new Label
            {
                Text = description,
                ForeColor = _muted,
                Dock = DockStyle.Fill,
                AutoSize = true,
                Margin = new Padding(gap.Left + 1, 0, gap.Right, 0)
            }, column, 2);
        }
    }

    private AppearanceOptions Selection => new(_island.Checked ? TaskbarLayout.Island : TaskbarLayout.FullWidth,
        (int)_radius.Value, (int)_transparency.Value, (int)_hoverLift.Value, (int)_clickDepth.Value);

    private async Task RunAsync(AppearanceOptions? layout)
    {
        _busy = true;
        RefreshButtons();
        try
        {
            if (layout is { } selection)
            {
                _status.Text = selection.Layout == TaskbarLayout.Windows ? "Restoring Windows appearance…" : "Applying the layout…";
                await _styles.ApplyAsync(selection);
            }

            (_current, _running, _savedWithMotion) = await _styles.ReadAsync();
            if (layout is null)
            {
                _fullWidth.Checked = _current.Layout != TaskbarLayout.Island;
                _island.Checked = _current.Layout == TaskbarLayout.Island;
                _radius.Value = _current.Radius;
                _transparency.Value = _current.Transparency;
                _hoverLift.Value = _current.HoverLift;
                _clickDepth.Value = _current.ClickDepth;
            }
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
            RefreshButtons();
        }
    }

    private void RefreshButtons()
    {
        _island.Enabled = _fullWidth.Enabled = !_busy;
        _radius.Enabled = _transparency.Enabled = _hoverLift.Enabled = _clickDepth.Enabled = !_busy;
        _apply.Enabled = _available && !_busy && (_current != Selection || !_running || !_savedWithMotion);
        _restore.Enabled = _available && !_busy && _current.Layout != TaskbarLayout.Windows;
    }

    private sealed class LayoutPreview(bool island, Color accent) : Control
    {
        private bool _selected;
        private int _radius = 8;
        private int _transparency = 40;
        private int _hoverLift = AppearanceOptions.DefaultHoverLift;
        private int _clickDepth = AppearanceOptions.DefaultClickDepth;
        private Point? _pointer;
        private bool _pressed;
        [System.ComponentModel.DesignerSerializationVisibility(System.ComponentModel.DesignerSerializationVisibility.Hidden)]
        public bool Selected
        {
            get => _selected;
            set { _selected = value; Invalidate(); }
        }

        public void SetGlass(int radius, int transparency)
        {
            _radius = radius;
            _transparency = transparency;
            Invalidate();
        }

        public void SetMotion(int hoverLift, int clickDepth)
        {
            _hoverLift = hoverLift;
            _clickDepth = clickDepth;
            Invalidate();
        }

        protected override void OnMouseMove(MouseEventArgs args)
        {
            base.OnMouseMove(args);
            _pointer = args.Location;
            Invalidate();
        }

        protected override void OnMouseLeave(EventArgs args)
        {
            base.OnMouseLeave(args);
            _pointer = null;
            _pressed = false;
            Invalidate();
        }

        protected override void OnMouseDown(MouseEventArgs args)
        {
            base.OnMouseDown(args);
            _pressed = args.Button == MouseButtons.Left;
            Invalidate();
        }

        protected override void OnMouseUp(MouseEventArgs args)
        {
            base.OnMouseUp(args);
            _pressed = false;
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs args)
        {
            base.OnPaint(args);
            if (Width < 10 || Height < 10)
                return;

            var canvas = args.Graphics;
            canvas.SmoothingMode = SmoothingMode.AntiAlias;
            var rect = new RectangleF(2, 2, Width - 5, Height - 5);
            using var frame = Rounded(rect, 11);
            using var wallpaper = new LinearGradientBrush(rect, Color.FromArgb(21, 52, 86), Color.FromArgb(117, 158, 172), 24);
            canvas.FillPath(wallpaper, frame);
            var state = canvas.Save();
            canvas.SetClip(frame);
            using var light = new SolidBrush(Color.FromArgb(52, 178, 231, 237));
            using var dark = new SolidBrush(Color.FromArgb(68, 7, 29, 68));
            canvas.FillEllipse(light, rect.X + rect.Width * .29f, rect.Y - rect.Height * .54f, rect.Width, rect.Height * 1.19f);
            canvas.FillEllipse(dark, rect.X - rect.Width * .31f, rect.Y + rect.Height * .2f, rect.Width * 1.25f, rect.Height * 1.48f);

            var dpi = DeviceDpi / 96f;
            var height = Math.Min(26 * dpi, rect.Height / 3);
            var width = island ? rect.Width * .69f : rect.Width;
            var bar = new RectangleF(rect.X + (rect.Width - width) / 2,
                rect.Bottom - height - (island ? 7 * dpi : 0), width, height);
            using var panel = Rounded(bar, _radius * .5f * dpi);
            using var glass = new SolidBrush(Color.FromArgb((int)(255 * (1 - _transparency / 100d)), 22, 32, 49));
            using var border = new Pen(Color.FromArgb(128, 185, 215, 234), dpi);
            canvas.FillPath(glass, panel);
            canvas.DrawPath(border, panel);
            DrawControls(canvas, bar, dpi);
            canvas.Restore(state);
            using var outline = new Pen(Selected ? accent : Color.FromArgb(185, 196, 210), Selected ? 2 * dpi : dpi);
            canvas.DrawPath(outline, frame);
        }

        private void DrawControls(Graphics canvas, RectangleF bar, float dpi)
        {
            var size = Math.Min(bar.Height * .39f, 11 * dpi);
            var y = bar.Y + (bar.Height - size) / 2;
            var x = island ? bar.Left + bar.Width * .09f : bar.Left + bar.Width * .36f;
            var cell = size * .43f;
            using var windows = new SolidBrush(Color.FromArgb(105, 197, 246));
            foreach (var column in new[] { 0, 1 })
                foreach (var row in new[] { 0, 1 })
                    canvas.FillRectangle(windows, x + column * size * .54f, y + row * size * .54f, cell, cell);

            x += size * 1.65f;
            using var glyph = new Pen(Color.FromArgb(224, 239, 250), 1.2f * dpi);
            canvas.DrawEllipse(glyph, x, y, size * .7f, size * .7f);
            canvas.DrawLine(glyph, x + size * .62f, y + size * .64f, x + size, y + size);
            x += size * 1.65f;
            foreach (var color in new[] { Color.FromArgb(229, 175, 108), Color.FromArgb(115, 126, 206), Color.FromArgb(91, 175, 169) })
            {
                var iconBounds = new RectangleF(x, y, size, size);
                var hit = RectangleF.Inflate(iconBounds, size * .25f, (bar.Height - size) / 2);
                if (_pointer is { } pointer && hit.Contains(pointer))
                {
                    var press = _pressed && _clickDepth > 0;
                    var delta = size * (press ? -_clickDepth * .03f : _hoverLift * .035f) / 2;
                    iconBounds.Inflate(delta, delta);
                    iconBounds.Y += (press ? _clickDepth : -_hoverLift) * dpi * .5f;
                }
                using var icon = Rounded(iconBounds, 2 * dpi);
                using var ink = new SolidBrush(color);
                canvas.FillPath(ink, icon);
                x += size * 1.54f;
            }
            var tray = bar.Right - size * 3.95f;
            for (var index = 0; index < 3; index++)
            {
                var line = index * 1.7f * dpi;
                canvas.DrawLine(glyph, tray + index * 2f * dpi, y + size - line, tray + index * 2f * dpi, y + size);
            }
            canvas.DrawLine(glyph, tray + size * 1.32f, y + size * .3f, bar.Right - size, y + size * .3f);
            canvas.DrawLine(glyph, tray + size * 1.52f, y + size * .74f, bar.Right - size, y + size * .74f);
        }

        private static GraphicsPath Rounded(RectangleF rect, float radius)
        {
            var path = new GraphicsPath();
            if (radius <= 0)
            {
                path.AddRectangle(rect);
                return path;
            }

            var diameter = Math.Min(radius * 2, Math.Min(rect.Width, rect.Height));
            path.AddArc(rect.Left, rect.Top, diameter, diameter, 180, 90);
            path.AddArc(rect.Right - diameter, rect.Top, diameter, diameter, 270, 90);
            path.AddArc(rect.Right - diameter, rect.Bottom - diameter, diameter, diameter, 0, 90);
            path.AddArc(rect.Left, rect.Bottom - diameter, diameter, diameter, 90, 90);
            path.CloseFigure();
            return path;
        }
    }
}
