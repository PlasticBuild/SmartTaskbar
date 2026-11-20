using System.Drawing.Drawing2D;
using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar.Appearance;

internal sealed class TaskbarSandbox : PictureBox
{
    private readonly Timer _frames = new() { Interval = 16 };
    private readonly List<RectangleF> _appSlots = [];
    private AppearanceTheme _theme;
    private AppearanceOptions _options = new(TaskbarLayout.Island);
    private RevealZone _zone;
    private TaskbarPreviewIcons? _icons;
    private RectangleF _display;
    private RectangleF _restingTaskbar;
    private Point? _pointer;
    private bool _mouseDown;
    private bool _fullscreen;
    private int _hover = -1;
    private long _edgeSince;
    private long _frameAt;
    private float _windowSize;
    private float _barVisible = 1;
    private float _islandAmount = 1;
    private bool _menuOpen;
    private bool _animateMenus;
    private float _menuAmount;

    public TaskbarSandbox()
    {
        SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint |
            ControlStyles.OptimizedDoubleBuffer | ControlStyles.ResizeRedraw, true);
        TabStop = false;
        Name = "TaskbarPreview";
        AccessibleRole = AccessibleRole.Graphic;
        AccessibleName = "Interactive preview of the Windows taskbar";
        QueryAccessibilityHelp += (_, args) => args.HelpString = AccessibleDescription;
        _frames.Tick += (_, _) => Animate();
    }

    public void SetTheme(AppearanceTheme theme)
    {
        _theme = theme;
        BackColor = theme.Card;
        Invalidate();
    }

    public void SetLayout(AppearanceOptions options, RevealZone zone)
    {
        if (_options == options && _zone == zone)
            return;
        var layoutChanged = _options.Layout != options.Layout;
        _options = options;
        _zone = zone;
        _edgeSince = 0;
        if (_fullscreen || layoutChanged)
            StartAnimation();
        Invalidate();
    }

    public void SetIcons(TaskbarPreviewIcons newIcons)
    {
        _icons?.Dispose();
        _icons = newIcons;
        AccessibleDescription = string.Join(", ", newIcons.Icons.Select(icon => icon.Name));
        Invalidate();
    }

    public string? IconSignature => _icons?.Signature;

    public void SetMenuAnimations(bool enabled)
    {
        if (_animateMenus == enabled) return;
        _animateMenus = enabled;
        if (!enabled)
        {
            _menuAmount = _menuOpen ? 1 : 0;
            Invalidate();
        }
    }

    public void ToggleMenu()
    {
        _menuOpen = !_menuOpen;
        if (!_animateMenus)
            _menuAmount = _menuOpen ? 1 : 0;
        StartAnimation();
        Invalidate();
    }

    public void SetFullscreen(bool fullscreen)
    {
        _fullscreen = fullscreen;
        if (_menuOpen)
            ToggleMenu();
        _edgeSince = 0;
        StartAnimation();
    }

    protected override void OnMouseMove(MouseEventArgs args)
    {
        base.OnMouseMove(args);
        _pointer = args.Location;
        UpdateHover();
        if (_fullscreen)
            StartAnimation();
    }

    protected override void OnMouseLeave(EventArgs args)
    {
        base.OnMouseLeave(args);
        _pointer = null;
        _mouseDown = false;
        _edgeSince = 0;
        UpdateHover();
        if (_fullscreen)
            StartAnimation();
    }

    protected override void OnMouseDown(MouseEventArgs args)
    {
        base.OnMouseDown(args);
        _pointer = args.Location;
        UpdateHover();
        if (args.Button == MouseButtons.Left && _hover >= 0)
        {
            _mouseDown = true;
            Invalidate();
        }
    }

    protected override void OnMouseUp(MouseEventArgs args)
    {
        base.OnMouseUp(args);
        if (_mouseDown)
        {
            _mouseDown = false;
            Invalidate();
        }
    }

    private void UpdateHover()
    {
        var current = _pointer is { } cursor ? _appSlots.FindIndex(slot => slot.Contains(cursor)) : -1;
        if (current == _hover)
            return;
        _hover = current;
        Invalidate();
    }

    private void StartAnimation()
    {
        if (_frames.Enabled)
            return;
        _frameAt = Environment.TickCount64;
        _frames.Start();
    }

    private void Animate()
    {
        var now = Environment.TickCount64;
        var delta = Math.Clamp((now - _frameAt) / 185f, .04f, .2f);
        _frameAt = now;

        var atEdge = _fullscreen && _pointer is { } point && TriggerBand().Contains(point);
        if (!atEdge)
            _edgeSince = 0;
        else if (_edgeSince == 0)
            _edgeSince = now;
        var hoveringBar = _barVisible > .98f && _windowSize > .98f &&
            _pointer is { } cursor && _restingTaskbar.Contains(cursor);
        var show = !_fullscreen || _menuOpen || hoveringBar || (atEdge && now - _edgeSince >= 150);
        var nextSize = Approach(_windowSize, _fullscreen ? 1 : 0, delta);
        var nextBar = Approach(_barVisible, show ? 1 : 0, delta);
        var nextIsland = Approach(_islandAmount, _options.Layout == TaskbarLayout.Island ? 1 : 0, delta);
        var nextMenu = Approach(_menuAmount, _menuOpen ? 1 : 0, delta);
        var changed = nextSize != _windowSize || nextBar != _barVisible || nextIsland != _islandAmount || nextMenu != _menuAmount;
        _windowSize = nextSize;
        _barVisible = nextBar;
        _islandAmount = nextIsland;
        _menuAmount = nextMenu;
        if (changed)
            Invalidate();
        else if (!atEdge || show)
            _frames.Stop();
    }

    private RectangleF TriggerBand()
    {
        var width = _zone switch
        {
            RevealZone.CenterHalf => _display.Width / 2,
            RevealZone.VisibleTaskbar => _restingTaskbar.Width,
            _ => _display.Width
        };
        var depth = 8 * DeviceDpi / 96f;
        var left = _zone == RevealZone.VisibleTaskbar ? _restingTaskbar.Left : _display.X + (_display.Width - width) / 2;
        return new RectangleF(left, _display.Bottom - depth, width, depth + 3);
    }

    private static float Approach(float value, float target, float step)
        => Math.Abs(value - target) <= step ? target : value + Math.Sign(target - value) * step;

    protected override void OnPaint(PaintEventArgs args)
    {
        var graphics = args.Graphics;
        graphics.Clear(BackColor);
        if (Width < LogicalToDeviceUnits(200) || Height < LogicalToDeviceUnits(75))
            return;

        graphics.SmoothingMode = SmoothingMode.AntiAlias;
        graphics.InterpolationMode = InterpolationMode.HighQualityBicubic;
        var bounds = new RectangleF(1, 1, Width - 3, Height - 3);
        var dpi = DeviceDpi / 96f;
        _display = bounds;
        using var screen = Rounded(bounds, 11 * dpi);
        using var outline = new Pen(_theme.Line, dpi);
        using var backdrop = new LinearGradientBrush(bounds, Color.FromArgb(19, 31, 51), Color.FromArgb(55, 90, 127), 32);
        graphics.FillPath(backdrop, screen);
        var state = graphics.Save();
        graphics.SetClip(screen);
        DrawWindow(graphics, bounds, dpi);
        DrawTaskbar(graphics, bounds, dpi);
        if (_fullscreen && _barVisible < .12f)
        {
            var edge = TriggerBand();
            using var guide = new SolidBrush(Color.FromArgb(155, _theme.Accent));
            graphics.FillRectangle(guide, edge.Left, bounds.Bottom - 2 * dpi, edge.Width, 2 * dpi);
        }
        graphics.Restore(state);
        graphics.DrawPath(outline, screen);
    }

    private void DrawWindow(Graphics graphics, RectangleF bounds, float dpi)
    {
        var extent = new RectangleF(bounds.Left + bounds.Width * .13f * (1 - _windowSize),
            bounds.Top + bounds.Height * .14f * (1 - _windowSize),
            bounds.Width * (.68f + .32f * _windowSize), bounds.Height * (.62f + .38f * _windowSize));
        using var window = Rounded(extent, 7 * dpi * (1 - _windowSize));
        using var fill = new SolidBrush(_theme.Dark ? Color.FromArgb(38, 47, 64) : Color.FromArgb(221, 231, 244));
        using var header = new SolidBrush(_theme.Dark ? Color.FromArgb(58, 72, 96) : Color.FromArgb(192, 207, 226));
        graphics.FillPath(fill, window);
        var save = graphics.Save();
        graphics.SetClip(window, CombineMode.Intersect);
        graphics.FillRectangle(header, extent.X, extent.Y, extent.Width, 21 * dpi);
        using var ink = new SolidBrush(_theme.Dark ? Color.FromArgb(87, 103, 130) : Color.FromArgb(163, 183, 208));
        graphics.FillRectangle(ink, extent.X + 13 * dpi, extent.Y + 41 * dpi, extent.Width * .17f, extent.Height * .58f);
        graphics.FillRectangle(ink, extent.X + extent.Width * .27f, extent.Y + 41 * dpi, extent.Width * .48f, 7 * dpi);
        graphics.FillRectangle(ink, extent.X + extent.Width * .27f, extent.Y + 57 * dpi, extent.Width * .33f, 5 * dpi);
        graphics.Restore(save);
    }

    private void DrawTaskbar(Graphics graphics, RectangleF bounds, float dpi)
    {
        var apps = _icons?.Icons ?? [];
        var search = _icons?.SearchMode ?? 0;
        var trayWidth = 101 * dpi;
        var searchWidth = search switch { 2 => 66 * dpi, 3 => 45 * dpi, > 0 => 24 * dpi, _ => 0 };
        var space = Math.Clamp((bounds.Width - trayWidth - 24 * dpi - searchWidth) / (apps.Length + 1),
            1, 27 * dpi);
        var group = space * (apps.Length + 1) + searchWidth;
        var depth = 33 * dpi;
        var islandWidth = Math.Min(bounds.Width - 10 * dpi, group + trayWidth + 13 * dpi);
        var barWidth = bounds.Width + (islandWidth - bounds.Width) * _islandAmount;
        var spaceBeside = bounds.Width - barWidth;
        var inset = Math.Min(spaceBeside / 2, 6 * dpi * _islandAmount);
        var leftInset = _options.Alignment switch
        {
            IslandAlignment.Left => inset,
            IslandAlignment.Right => spaceBeside - inset,
            _ => spaceBeside / 2
        };
        _restingTaskbar = new RectangleF(bounds.X + leftInset,
            bounds.Bottom - depth - 4 * dpi * _islandAmount, barWidth, depth);
        var bar = _restingTaskbar;
        bar.Y += (depth + 5 * dpi) * (1 - _barVisible);
        if (_menuAmount > 0)
            DrawPreviewMenu(graphics, bar, dpi, apps);
        using var shape = Rounded(bar, _options.Radius * .74f * dpi);
        var alpha = (int)Math.Clamp(248 - _options.Transparency * 1.25, 142, 248);
        using var glass = new SolidBrush(Color.FromArgb(alpha, _theme.Dark ? Color.FromArgb(21, 26, 36) : Color.FromArgb(234, 240, 248)));
        using var rim = new Pen(Color.FromArgb(108, _theme.Dark ? Color.White : Color.FromArgb(73, 103, 137)), dpi);
        graphics.FillPath(glass, shape);
        graphics.DrawPath(rim, shape);

        var fullWidthInset = Math.Max(0, Math.Min((bounds.Width - group) / 2, bounds.Width - trayWidth - group));
        var x = bar.Left + fullWidthInset + (6 * dpi - fullWidthInset) * _islandAmount;
        var iconSize = Math.Min(18 * dpi, space * .67f);
        var y = bar.Y + (depth - iconSize) / 2;
        DrawStart(graphics, x + (space - iconSize) / 2, y, iconSize);
        x += space;
        if (search > 0)
        {
            using var searchInk = new Pen(_theme.Text, Math.Max(1, dpi * 1.2f));
            var left = x + 5 * dpi;
            if (search > 1)
            {
                using var field = Rounded(new RectangleF(x, bar.Y + 6 * dpi, searchWidth - 3 * dpi, 21 * dpi), 9 * dpi);
                using var tint = new SolidBrush(Color.FromArgb(38, _theme.Text));
                graphics.FillPath(tint, field);
                TextRenderer.DrawText(graphics, "Search", Font,
                    new Rectangle((int)(left + iconSize), (int)bar.Y, (int)(searchWidth - iconSize), (int)depth),
                    _theme.Text, TextFormatFlags.Left | TextFormatFlags.VerticalCenter |
                        TextFormatFlags.NoPadding | TextFormatFlags.PreserveGraphicsClipping | TextFormatFlags.EndEllipsis);
            }
            graphics.DrawEllipse(searchInk, left, y + iconSize * .1f, iconSize * .55f, iconSize * .55f);
            graphics.DrawLine(searchInk, left + iconSize * .52f, y + iconSize * .62f, left + iconSize * .76f, y + iconSize * .87f);
            x += searchWidth;
        }

        _appSlots.Clear();
        for (var index = 0; index < apps.Length; index++)
        {
            _appSlots.Add(_barVisible > .8f ? new RectangleF(x, bar.Y, space, bar.Height) : RectangleF.Empty);
            var icon = new RectangleF(x + (space - iconSize) / 2, y, iconSize, iconSize);
            if (_hover == index)
            {
                var effect = _mouseDown && _options.ClickDepth != 0 ? -_options.ClickDepth : _options.HoverLift;
                icon.Inflate(iconSize * effect * .015f, iconSize * effect * .015f);
                icon.Y -= effect * dpi * .65f;
            }
            graphics.DrawImage(apps[index].Image, icon);
            x += space;
        }
        DrawTray(graphics, new RectangleF(bar.Right - trayWidth, bar.Y, trayWidth, depth), dpi);
    }

    private void DrawPreviewMenu(Graphics graphics, RectangleF bar, float dpi, PreviewIcon[] apps)
    {
        var width = Math.Min(177 * dpi, bar.Width);
        var height = Math.Min(75 * dpi, _display.Height / 2);
        var eased = 1 - (1 - _menuAmount) * (1 - _menuAmount);
        var bounds = new RectangleF(bar.Left + (bar.Width - width) / 2, bar.Top - height * eased, width, height);
        var save = graphics.Save();
        graphics.SetClip(new RectangleF(bounds.X, bounds.Y, bounds.Width, height * eased), CombineMode.Intersect);
        using var path = Rounded(bounds, (_options.Radius + 2) * dpi);
        using var material = new SolidBrush(Color.FromArgb(245 - _options.Transparency, _theme.Input));
        using var edge = new Pen(_theme.Line, dpi);
        graphics.FillPath(material, path);
        graphics.DrawPath(edge, path);
        TextRenderer.DrawText(graphics, "Start", Font, Rectangle.Round(new RectangleF(
            bounds.X + 13 * dpi, bounds.Y + 7 * dpi, width - 26 * dpi, 23 * dpi)), _theme.Text,
            TextFormatFlags.Left | TextFormatFlags.VerticalCenter | TextFormatFlags.PreserveGraphicsClipping);
        for (var i = 0; i < Math.Min(5, apps.Length); i++)
            graphics.DrawImage(apps[i].Image, new RectangleF(bounds.X + (14 + i * 31) * dpi,
                bounds.Y + 38 * dpi, 20 * dpi, 20 * dpi));
        graphics.Restore(save);
    }

    private void DrawTray(Graphics graphics, RectangleF tray, float dpi)
    {
        var state = graphics.Save();
        graphics.TranslateTransform(tray.X, tray.Y + tray.Height / 2);
        graphics.ScaleTransform(dpi, dpi);
        using var ink = new Pen(_theme.Text, 1.1f) { StartCap = LineCap.Round, EndCap = LineCap.Round };
        using var fill = new SolidBrush(_theme.Text);
        graphics.DrawLines(ink, [new PointF(2, 1), new PointF(5, -2), new PointF(8, 1)]);
        graphics.DrawArc(ink, 13, -4, 12, 11, 222, 96);
        graphics.DrawArc(ink, 16, -1, 6, 6, 222, 96);
        graphics.FillEllipse(fill, 18.3f, 3, 1.4f, 1.4f);
        graphics.DrawPolygon(ink, [new PointF(32, -1.5f), new PointF(34, -1.5f), new PointF(37, -4),
            new PointF(37, 4), new PointF(34, 1.5f), new PointF(32, 1.5f)]);
        graphics.DrawArc(ink, 35, -4, 8, 8, -52, 104);
        graphics.DrawRectangle(ink, 49, -3, 10, 6);
        graphics.DrawLine(ink, 61, -1, 61, 1);
        graphics.FillRectangle(fill, 50.5f, -1.5f, 6, 3);
        graphics.Restore(state);

        TextRenderer.DrawText(graphics, DateTime.Now.ToString("h:mm"), Font,
            new Rectangle((int)(tray.X + 66 * dpi), (int)tray.Y, (int)(tray.Width - 68 * dpi), (int)tray.Height),
            _theme.Text, TextFormatFlags.Left | TextFormatFlags.VerticalCenter |
                TextFormatFlags.NoPadding | TextFormatFlags.PreserveGraphicsClipping);
    }

    private static void DrawStart(Graphics graphics, float x, float y, float size)
    {
        using var blue = new SolidBrush(Color.FromArgb(100, 188, 248));
        var cell = size * .43f;
        var span = size * .55f;
        graphics.FillRectangle(blue, x, y, cell, cell);
        graphics.FillRectangle(blue, x + span, y, cell, cell);
        graphics.FillRectangle(blue, x, y + span, cell, cell);
        graphics.FillRectangle(blue, x + span, y + span, cell, cell);
    }

    private static GraphicsPath Rounded(RectangleF rect, float radius)
    {
        var path = new GraphicsPath();
        if (radius <= 0)
        {
            path.AddRectangle(rect);
            return path;
        }
        var size = Math.Min(radius * 2, Math.Min(rect.Width, rect.Height));
        path.AddArc(rect.Left, rect.Top, size, size, 180, 90);
        path.AddArc(rect.Right - size, rect.Top, size, size, 270, 90);
        path.AddArc(rect.Right - size, rect.Bottom - size, size, size, 0, 90);
        path.AddArc(rect.Left, rect.Bottom - size, size, size, 90, 90);
        path.CloseFigure();
        return path;
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            _frames.Dispose();
            _icons?.Dispose();
        }
        base.Dispose(disposing);
    }
}
