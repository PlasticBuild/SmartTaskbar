using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar;

internal enum IslandMenuSurface { Start, Search, QuickSettings, Notifications, Overflow, JumpList }

/// <summary>Paints just the edge where an open shell flyout meets the island.</summary>
internal sealed class IslandMenuJoin : NativeWindow, IDisposable
{
    private readonly Timer _frames = new() { Interval = 16 };
    private IntPtr _window;
    private IntPtr _memoryDc;
    private IntPtr _dib;
    private IntPtr _previousBitmap;
    private Bitmap? _pixels;
    private Rectangle _island, _contact, _layer;
    private int _dpi, _radius, _anchor, _duration, _gap;
    private Color _face, _tray;
    private float _amount, _from, _to;
    private long _start;
    private bool _visible;

    internal Color? MaterialColor => _face.IsEmpty ? null : _face;

    public IslandMenuJoin() => _frames.Tick += (_, _) => Advance();

    public void Apply(IntPtr window, TaskbarInfo bar, Rectangle island, Rectangle settledWrapper,
        IslandMenuSurface kind, int durationMs, int radiusDip)
    {
        if (window == IntPtr.Zero || SystemInformation.HighContrast || island.Width <= 0 || settledWrapper.Width <= 0)
        {
            Close(false);
            return;
        }

        var dpi = (int)Fun.GetDpiForWindow(bar.Handle);
        if (dpi <= 0)
            dpi = 96;
        var radius = Math.Clamp(radiusDip, 0, 24) * dpi / 96;
        var menu = Face(kind, settledWrapper, island, bar.Display, dpi);
        var contact = Rectangle.FromLTRB(Math.Max(island.Left, menu.Left), island.Top + Scale(4, dpi),
            Math.Min(island.Right, menu.Right), island.Bottom);
        // The transparent shadow is part of the shell HWND. Cover the space from
        // the native panel's painted bottom to the island without moving either.
        var gap = Math.Max(0, contact.Top - (menu.Bottom - Scale(14, dpi)));
        if (gap > Scale(48, dpi) || contact.Width < Scale(8, dpi) || contact.Height <= 0)
        {
            Close(false);
            return;
        }

        _duration = Math.Clamp(durationMs, 0, 600);
        if (window == _window && _island == island && _contact == contact && _radius == radius && _dpi == dpi && _gap == gap)
            return;
        var opening = window != _window || _to == 0;
        _dpi = dpi;
        _radius = radius;
        _island = island;
        _contact = contact;
        _gap = gap;
        _window = window;
        _layer = Rectangle.FromLTRB(island.Left, contact.Top - Math.Max(radius, gap) - Scale(3, dpi),
            island.Right, contact.Top + radius + Scale(3, dpi));
        if (!EnsureBuffer())
        {
            Close(false);
            return;
        }

        _anchor = kind switch
        {
            IslandMenuSurface.Start => island.Left + Scale(24, dpi),
            IslandMenuSurface.Search => island.Left + Scale(60, dpi),
            IslandMenuSurface.Notifications => island.Right - Scale(38, dpi),
            _ => menu.Left + menu.Width / 2
        };
        if (Fun.GetCursorPos(out var pointer) && island.Contains(pointer.x, pointer.y))
            _anchor = pointer.x;
        _anchor = Math.Clamp(_anchor, contact.Left, contact.Right);
        SampleMaterial();
        if (opening)
        {
            _amount = 0;
            AnimateTo(1);
        }
        else
            PaintLayer();
    }

    private static int Scale(int dips, int dpi) => (dips * dpi + 48) / 96;

    internal static Rectangle Face(IslandMenuSurface kind, Rectangle wrapper, Rectangle island, Rectangle display, int dpi)
    {
        var gutter = Scale(12, dpi);
        var width = kind switch
        {
            IslandMenuSurface.Start => Scale(IslandMenuWidth.Measure(island.Width, wrapper.Width, (uint)dpi), dpi),
            IslandMenuSurface.Search => Scale(600, dpi) - 2 * gutter,
            IslandMenuSurface.QuickSettings => Scale(344, dpi),
            IslandMenuSurface.Notifications => Scale(332, dpi),
            _ => wrapper.Width - 2 * gutter
        };
        width = Math.Clamp(width, 1, wrapper.Width);
        var x = kind switch
        {
            IslandMenuSurface.Start => IslandMenuWidth.StartFaceLeft(wrapper, island, width, dpi),
            IslandMenuSurface.Search when island.Left - display.Left <= Scale(IslandMenuWidth.SideInset + 2, dpi) =>
                wrapper.Left + gutter,
            IslandMenuSurface.Search when display.Right - island.Right <= Scale(IslandMenuWidth.SideInset + 2, dpi) =>
                wrapper.Right - gutter - width,
            IslandMenuSurface.Search => wrapper.Left + (wrapper.Width - width) / 2,
            IslandMenuSurface.QuickSettings or IslandMenuSurface.Notifications => wrapper.Right - gutter - width,
            _ => wrapper.Left + gutter
        };
        return new Rectangle(x, wrapper.Top, width, wrapper.Height);
    }

    private void SampleMaterial()
    {
        var dc = GetDC(IntPtr.Zero);
        if (dc == IntPtr.Zero)
            return;
        try
        {
            var x = _contact.Left + _contact.Width / 2;
            _tray = Sample(dc, x, _contact.Top + Scale(4, _dpi), Fun.IsLightTheme() ? Color.FromArgb(225, 225, 225) : Color.FromArgb(33, 33, 33));
            var menuY = _contact.Top - _gap - Scale(4, _dpi);
            var hit = Fun.GetAncestor(Fun.WindowFromPhysicalPoint(new TagPoint { x = x, y = menuY }), 2);
            _face = hit == _window ? Sample(dc, x, menuY, _tray) : _tray;
        }
        finally { ReleaseDC(IntPtr.Zero, dc); }
    }

    private static Color Sample(IntPtr dc, int x, int y, Color fallback)
    {
        var pixel = GetPixel(dc, x, y);
        return pixel == uint.MaxValue ? fallback : Color.FromArgb((int)(pixel & 255), (int)((pixel >> 8) & 255), (int)((pixel >> 16) & 255));
    }

    public void Close(bool animate)
    {
        if (_window == IntPtr.Zero && _to == 0 && (animate || !_frames.Enabled))
            return;
        _window = IntPtr.Zero;
        if (animate && _duration > 0 && _amount > 0)
            AnimateTo(0);
        else
        {
            _frames.Stop();
            _to = _amount = 0;
            HideLayer();
        }
    }

    private void AnimateTo(float target)
    {
        _from = _amount;
        _to = target;
        _start = Environment.TickCount64;
        if (_duration == 0 || _from == _to)
        {
            _amount = target;
            _frames.Stop();
            if (target == 1) PaintLayer(); else HideLayer();
            return;
        }
        _frames.Start();
        Advance();
    }

    private void Advance()
    {
        var elapsed = Math.Clamp((Environment.TickCount64 - _start) / (float)Math.Max(_duration, 1), 0f, 1f);
        var eased = elapsed * elapsed * (3 - 2 * elapsed);
        _amount = _from + (_to - _from) * eased;
        if (elapsed >= 1)
        {
            _frames.Stop();
            if (_to == 0) { HideLayer(); return; }
            SampleMaterial();
        }
        PaintLayer();
    }

    private void PaintLayer()
    {
        if (_pixels is null || _amount <= 0)
            return;
        var inset = Scale(1, _dpi);
        var y = _contact.Top - _layer.Top;
        var l = _anchor + (_contact.Left - _anchor) * _amount - _layer.Left;
        var r = _anchor + (_contact.Right - _anchor) * _amount - _layer.Left;
        if (r - l < 1)
            return;
        var radius = Math.Min(_radius * _amount, (r - l) / 2);
        var sharedLeft = (_island.Left + inset * 2 >= _contact.Left) && _amount > .98f;
        var sharedRight = (_island.Right - inset * 2 <= _contact.Right) && _amount > .98f;
        var leftRadius = sharedLeft ? 0 : Math.Min(radius, l);
        var rightRadius = sharedRight ? 0 : Math.Min(radius, _layer.Width - r);
        using var g = Graphics.FromImage(_pixels);
        g.Clear(Color.Transparent);
        g.SmoothingMode = SmoothingMode.AntiAlias;
        using var shape = new GraphicsPath();
        shape.StartFigure();
        shape.AddLine(l - leftRadius, y + inset, l - leftRadius, y);
        if (leftRadius > 0)
            shape.AddBezier(l - leftRadius, y, l - leftRadius * .45f, y, l, y - leftRadius * .45f, l, y - leftRadius);
        var menuEdge = y - _gap - inset;
        shape.AddLine(l, y - leftRadius, l, menuEdge);
        shape.AddLine(l, menuEdge, r, menuEdge);
        shape.AddLine(r, menuEdge, r, y - rightRadius);
        if (rightRadius > 0)
            shape.AddBezier(r, y - rightRadius, r, y - rightRadius * .45f, r + rightRadius * .45f, y, r + rightRadius, y);
        shape.AddLine(r + rightRadius, y, r + rightRadius, y + inset);
        shape.CloseFigure();
        using var glass = new LinearGradientBrush(new Point(0, Math.Max(0, y - Math.Max(_radius, _gap) - 1)), new Point(0, y + inset), _face, _tray);
        g.FillPath(glass, shape);
        using var trayBrush = new SolidBrush(_tray);
        if (radius > 0 && sharedLeft)
            g.FillRectangle(trayBrush, l, y, radius, radius);
        if (radius > 0 && sharedRight)
            g.FillRectangle(trayBrush, r - radius, y, radius, radius);
        g.Flush();
        var origin = _layer.Location;
        var size = _layer.Size;
        var source = Point.Empty;
        var blend = new Blend { ConstantAlpha = (byte)(255 * _amount), AlphaFormat = 1 };
        if (UpdateLayeredWindow(Handle, IntPtr.Zero, ref origin, ref size, _memoryDc, ref source, 0, ref blend, 2) && !_visible)
        {
            ShowWindow(Handle, 4);
            _visible = true;
        }
    }

    private bool EnsureBuffer()
    {
        if (Handle == IntPtr.Zero)
        {
            CreateHandle(new CreateParams
            {
                Caption = "SmartTaskbar menu join", Style = unchecked((int)0x80000000),
                ExStyle = 0x80000 | 0x08000000 | 0x20 | 0x80 | 0x8
            });
        }
        if (_pixels?.Size == _layer.Size)
            return true;
        ReleasePixels();
        _memoryDc = CreateCompatibleDC(IntPtr.Zero);
        var info = new BitmapInfo
        {
            HeaderSize = 40, Width = _layer.Width, Height = -_layer.Height, Planes = 1, BitCount = 32
        };
        _dib = CreateDIBSection(_memoryDc, ref info, 0, out var bits, IntPtr.Zero, 0);
        if (_memoryDc == IntPtr.Zero || _dib == IntPtr.Zero || bits == IntPtr.Zero)
        {
            ReleasePixels();
            return false;
        }
        _previousBitmap = SelectObject(_memoryDc, _dib);
        _pixels = new Bitmap(_layer.Width, _layer.Height, _layer.Width * 4, PixelFormat.Format32bppPArgb, bits);
        return true;
    }

    private void ReleasePixels()
    {
        _pixels?.Dispose();
        _pixels = null;
        if (_previousBitmap != IntPtr.Zero) SelectObject(_memoryDc, _previousBitmap);
        if (_dib != IntPtr.Zero) Fun.DeleteObject(_dib);
        if (_memoryDc != IntPtr.Zero) DeleteDC(_memoryDc);
        _memoryDc = _dib = _previousBitmap = IntPtr.Zero;
    }

    private void HideLayer()
    {
        if (_visible)
        {
            ShowWindow(Handle, 0);
            _visible = false;
        }
        ReleasePixels();
    }

    protected override void WndProc(ref Message message)
    {
        if (message.Msg == 0x0084) { message.Result = new IntPtr(-1); return; }
        if (message.Msg == 0x0021) { message.Result = new IntPtr(3); return; }
        base.WndProc(ref message);
    }

    public void Dispose()
    {
        _frames.Dispose();
        ReleasePixels();
        if (Handle != IntPtr.Zero) DestroyHandle();
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BitmapInfo
    {
        public int HeaderSize, Width, Height;
        public ushort Planes, BitCount;
        public uint Compression, Bytes;
        public int XPels, YPels;
        public uint Used, Important;
    }

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    private struct Blend { public byte Operation, Flags, ConstantAlpha, AlphaFormat; }

    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
    [DllImport("gdi32.dll")] private static extern uint GetPixel(IntPtr dc, int x, int y);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] private static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] private static extern IntPtr SelectObject(IntPtr dc, IntPtr value);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateDIBSection(IntPtr dc, ref BitmapInfo info,
        uint usage, out IntPtr bits, IntPtr section, uint offset);
    [DllImport("user32.dll")] private static extern bool UpdateLayeredWindow(IntPtr window, IntPtr dc,
        ref Point dest, ref Size size, IntPtr sourceDc, ref Point source, uint key, ref Blend blend, uint flags);
}
