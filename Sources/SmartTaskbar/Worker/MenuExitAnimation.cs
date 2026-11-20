using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Diagnostics;
using System.Runtime.InteropServices;
using Timer = System.Windows.Forms.Timer;

namespace SmartTaskbar;

/// <summary>Animates the shell panel's exit, using its material when its final frame is unavailable.</summary>
internal sealed class MenuExitAnimation : IDisposable
{
    private const int DurationMs = 155;
    private readonly Timer _settle = new() { Interval = 230 };
    private readonly Timer _fallback = new() { Interval = 32 };
    private readonly Timer _frames = new() { Interval = 15 };
    private IntPtr _window;
    private int _process, _generation, _dpi;
    private Rectangle _wrapper, _face, _visibleFace, _animationBounds;
    private long _started;
    private string _className = "";
    private Rectangle _local;
    private Color? _material;
    private int _radius;
    private bool _busy, _disposed;
    private ExitWindow? _layer;

    public MenuExitAnimation()
    {
        _settle.Tick += (_, _) => MeasureVisibleFace();
        _fallback.Tick += (_, _) => PlayMaterial();
        _frames.Tick += (_, _) => DrawFrame();
    }

    public void Observe(IntPtr window, Rectangle wrapper, Rectangle face, int dpi, int radiusDip, Color? material)
    {
        if (_disposed || window == IntPtr.Zero || face.Width < 20 || face.Height < 20)
            return;
        _material = material;
        _radius = Math.Clamp(radiusDip, 0, 24);
        if (window == _window && wrapper == _wrapper && face == _face)
            return;
        Close(false);
        _window = window;
        if (Fun.GetWindowThreadProcessId(window, out _process) == 0) return;
        _className = Fun.GetClassName(window);
        _wrapper = wrapper;
        _face = Rectangle.Intersect(face, wrapper);
        _dpi = Math.Clamp(dpi, 72, 480);
        _settle.Start();
    }

    private void MeasureVisibleFace()
    {
        _settle.Stop();
        if (_disposed || _window == IntPtr.Zero || !Fun.IsWindowVisible(_window) ||
            !Fun.GetWindowRect(_window, out var native))
            return;

        var window = _window;
        var actual = native.ToRectangle();
        if (actual.Size != _wrapper.Size || _face.Width <= 0 || _face.Height <= 0)
            return;
        var visible = _face;
        visible.Offset(actual.Left - _wrapper.Left, actual.Top - _wrapper.Top);
        visible = HitFace(window, visible, _dpi);
        if (visible.IsEmpty) return;

        _local = new Rectangle(visible.Left - actual.Left, visible.Top - actual.Top, visible.Width, visible.Height);
        _visibleFace = visible;
    }

    private static Rectangle HitFace(IntPtr window, Rectangle bounds, int dpi)
    {
        // Many XAML hosts span the whole monitor. Limit the cached material to the
        // panel under these points; the desktop DC is never used for pixel capture.
        var x = bounds.Left + bounds.Width / 2;
        var step = Math.Max(4, 8 * dpi / 96);
        var first = int.MaxValue;
        var last = int.MinValue;
        for (var y = bounds.Bottom - 2; y >= bounds.Top; y -= step)
        {
            var hit = Fun.WindowFromPhysicalPoint(new TagPoint { x = x, y = y });
            if (Fun.GetAncestor(hit, 2) != window) continue;
            first = Math.Min(first, y);
            last = Math.Max(last, y);
        }
        if (last <= first + step * 2) return Rectangle.Empty;
        return Rectangle.FromLTRB(bounds.Left, Math.Max(bounds.Top, first), bounds.Right, Math.Min(bounds.Bottom, last + 1));
    }

    private static bool CaptureUnavailable(Exception error) => error is ExternalException or
        System.ComponentModel.Win32Exception or ArgumentException or OutOfMemoryException or InvalidOperationException;

    private static Bitmap? Capture(IntPtr window, int expectedProcess, string expectedClass, Rectangle local)
    {
        using var host = Process.GetProcessById(expectedProcess);
        if (!IsWindowsShellHost(host.MainModule?.FileName)) return null;
        if (Fun.GetWindowThreadProcessId(window, out var process) == 0 || process != expectedProcess ||
            Fun.GetClassName(window) != expectedClass ||
            SendMessageTimeout(window, 0, IntPtr.Zero, IntPtr.Zero, 2, 80, out _) == IntPtr.Zero)
            return null;

        // At most 3 MiB of menu pixels per bitmap. PrintWindow targets only this
        // shell HWND. A slow shell can occupy one worker, never the animation thread.
        var scale = Math.Min(1, Math.Min(1024d / local.Width, 768d / local.Height));
        var width = Math.Max(1, (int)Math.Round(local.Width * scale));
        var height = Math.Max(1, (int)Math.Round(local.Height * scale));
        var image = new Bitmap(width, height, PixelFormat.Format32bppRgb);
        try
        {
            bool rendered;
            using (var g = Graphics.FromImage(image))
            {
                g.Clear(Color.Magenta);
                var dc = g.GetHdc();
                try
                {
                    SetMapMode(dc, 8);
                    SetWindowExtEx(dc, local.Width, local.Height, IntPtr.Zero);
                    SetViewportExtEx(dc, width, height, IntPtr.Zero);
                    SetWindowOrgEx(dc, local.Left, local.Top, IntPtr.Zero);
                    rendered = PrintWindow(window, dc, 2);
                }
                finally { g.ReleaseHdc(dc); }
            }
            if (rendered && !host.HasExited && Fun.GetWindowThreadProcessId(window, out process) != 0 &&
                process == expectedProcess && Fun.GetClassName(window) == expectedClass && HasContent(image))
                return image;
        }
        catch
        {
            image.Dispose();
            throw;
        }
        image.Dispose();
        return null;
    }

    private static bool IsWindowsShellHost(string? executable)
    {
        if (string.IsNullOrEmpty(executable)) return false;
        var path = Path.GetFullPath(executable);
        var windows = Environment.GetFolderPath(Environment.SpecialFolder.Windows);
        if (string.Equals(path, Path.Combine(windows, "explorer.exe"), StringComparison.OrdinalIgnoreCase) ||
            string.Equals(path, Path.Combine(Environment.SystemDirectory, "ShellHost.exe"), StringComparison.OrdinalIgnoreCase))
            return true;
        return path.StartsWith(Path.Combine(windows, "SystemApps") + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase) &&
            Path.GetFileName(path) is "StartMenuExperienceHost.exe" or "SearchHost.exe" or "ShellExperienceHost.exe";
    }

    private static bool HasContent(Bitmap image)
    {
        var minimum = 255;
        var maximum = 0;
        var material = 0;
        var untouched = 0;
        for (var row = 1; row < 32; row++)
        for (var col = 1; col < 24; col++)
        {
            var c = image.GetPixel(col * image.Width / 24, row * image.Height / 32);
            if (c.R > 245 && c.B > 245 && c.G < 10) { untouched++; continue; }
            var intensity = (c.R + c.G + c.B) / 3;
            if (intensity > 5 && intensity < 250) material++;
            minimum = Math.Min(minimum, intensity);
            maximum = Math.Max(maximum, intensity);
        }
        return untouched < 5 && material > 170 && maximum - minimum >= 28;
    }

    public void Close(bool animate)
    {
        _generation++;
        var source = _window;
        var local = _local;
        _local = Rectangle.Empty;
        _window = IntPtr.Zero;
        _settle.Stop();
        StopLayer();
        _fallback.Stop();
        if (!animate || source == IntPtr.Zero || _disposed || SystemInformation.HighContrast || !MenuAnimationSettings.Read())
        {
            _visibleFace = Rectangle.Empty;
            return;
        }

        // Never retain an opening image. Notifications, Search, and subpages can
        // all change the content independently of the native window geometry.
        if (_visibleFace.IsEmpty) _visibleFace = _face;
        if (_busy || local.IsEmpty)
            PlayMaterial();
        else
        {
            _fallback.Start();
            CaptureExit(source, _process, _className, local, _generation);
        }
    }

    private async void CaptureExit(IntPtr window, int process, string className, Rectangle local, int generation)
    {
        var start = Environment.TickCount64;
        _busy = true;
        Bitmap? image = null;
        try
        {
            image = await Task.Run(() => Capture(window, process, className, local));
            if (_disposed || generation != _generation || !_fallback.Enabled) return;
            if (image is null || Environment.TickCount64 - start > 32)
            {
                PlayMaterial();
                return;
            }
            _fallback.Stop();
            Play(image, _visibleFace);
            image = null;
        }
        catch (Exception error) when (CaptureUnavailable(error))
        {
            if (!_disposed && generation == _generation && _fallback.Enabled) PlayMaterial();
        }
        finally
        {
            image?.Dispose();
            _busy = false;
        }
    }

    private void PlayMaterial()
    {
        _fallback.Stop();
        if (_disposed || _visibleFace.IsEmpty || SystemInformation.HighContrast || !MenuAnimationSettings.Read()) return;
        // Collapse the panel material into its contact edge when the shell returns
        // no final frame. Only this shallow strip overlays the application.
        var depth = Math.Min(_visibleFace.Height, Math.Max(1, 64 * _dpi / 96));
        var bounds = Rectangle.FromLTRB(_visibleFace.Left, _face.Bottom - depth, _visibleFace.Right, _face.Bottom);
        Play(null, bounds);
    }

    private void Play(Bitmap? image, Rectangle bounds)
    {
        try
        {
            _animationBounds = bounds;
            _layer = new ExitWindow(image, bounds, _dpi, _radius, _material);
            _started = Environment.TickCount64;
            _layer.Show();
            _frames.Start();
        }
        catch (System.ComponentModel.Win32Exception)
        {
            if (_layer is null) image?.Dispose();
            StopLayer();
        }
    }

    private void DrawFrame()
    {
        var t = Math.Clamp((Environment.TickCount64 - _started) / (double)DurationMs, 0, 1);
        if (_layer is null || t >= 1) { StopLayer(); return; }
        var frame = _animationBounds;
        var travel = _layer.MaterialOnly
            ? (int)Math.Round((frame.Height - 1) * (1 - Math.Pow(1 - t, 2)))
            : (int)Math.Round(_dpi / 96d * 12 * t * t);
        frame.Y += travel;
        frame.Height = Math.Max(1, frame.Height - travel);
        _layer.Bounds = frame;
        _layer.Opacity = Math.Max(.01, (_layer.MaterialOnly ? .83 : .97) * (1 - t * t));
    }

    private void StopLayer()
    {
        _frames.Stop();
        _layer?.Dispose();
        _layer = null;
    }

    public void Dispose()
    {
        _disposed = true;
        Close(false);
        _settle.Dispose();
        _fallback.Dispose();
        _frames.Dispose();
    }

    private sealed class ExitWindow : Form
    {
        private readonly Bitmap? _image;
        private readonly Color _glass, _edge;
        private readonly int _radius;
        public bool MaterialOnly => _image is null;
        private readonly Size _original;

        public ExitWindow(Bitmap? image, Rectangle bounds, int dpi, int radiusDip, Color? material)
        {
            _image = image;
            _original = bounds.Size;
            var light = Fun.IsLightTheme();
            _glass = material ?? (light ? Color.FromArgb(232, 232, 232) : Color.FromArgb(40, 40, 40));
            _edge = light ? Color.White : Color.FromArgb(85, 85, 85);
            _radius = radiusDip * dpi / 96;
            FormBorderStyle = FormBorderStyle.None;
            AutoScaleMode = AutoScaleMode.None;
            StartPosition = FormStartPosition.Manual;
            Text = MaterialOnly ? "SmartTaskbar menu exit material" : "SmartTaskbar menu exit";
            ShowInTaskbar = false;
            TopMost = true;
            DoubleBuffered = true;
            AllowTransparency = true;
            ResizeRedraw = true;
            if (MaterialOnly)
            {
                BackColor = TransparencyKey = Color.Magenta;
            }
            Opacity = MaterialOnly ? .83 : .97;
            Bounds = bounds;
        }

        protected override bool ShowWithoutActivation => true;
        protected override CreateParams CreateParams
        {
            get
            {
                var settings = base.CreateParams;
                settings.ExStyle |= 0x08000000 | 0x20 | 0x80; // NOACTIVATE, TRANSPARENT, TOOLWINDOW
                return settings;
            }
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            if (_image is not null)
            {
                e.Graphics.DrawImage(_image, new Rectangle(Point.Empty, _original));
                return;
            }
            var bounds = ClientRectangle;
            if (bounds.Width < 2 || bounds.Height < 2) return;
            var diameter = Math.Min(2 * _radius, Math.Min(bounds.Width - 1, bounds.Height - 1));
            using var brush = new SolidBrush(_glass);
            if (diameter < 2)
            {
                e.Graphics.FillRectangle(brush, bounds);
                return;
            }
            using var outline = new GraphicsPath();
            outline.AddArc(0, 0, diameter, diameter, 180, 90);
            outline.AddArc(bounds.Width - diameter - 1, 0, diameter, diameter, 270, 90);
            outline.AddLine(bounds.Width - 1, bounds.Height, 0, bounds.Height);
            outline.CloseFigure();
            using var pen = new Pen(_edge);
            e.Graphics.FillPath(brush, outline);
            e.Graphics.DrawPath(pen, outline);
        }

        protected override void WndProc(ref Message m)
        {
            if (m.Msg == 0x84) { m.Result = new IntPtr(-1); return; }
            if (m.Msg == 0x21) { m.Result = new IntPtr(3); return; }
            base.WndProc(ref m);
        }
        protected override void Dispose(bool disposing)
        {
            if (disposing) _image?.Dispose();
            base.Dispose(disposing);
        }
    }

    [DllImport("user32.dll")] private static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW")]
    private static extern IntPtr SendMessageTimeout(IntPtr window, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
    [DllImport("gdi32.dll")] private static extern int SetMapMode(IntPtr dc, int mode);
    [DllImport("gdi32.dll")] private static extern bool SetWindowExtEx(IntPtr dc, int width, int height, IntPtr old);
    [DllImport("gdi32.dll")] private static extern bool SetViewportExtEx(IntPtr dc, int width, int height, IntPtr old);
    [DllImport("gdi32.dll")] private static extern bool SetWindowOrgEx(IntPtr dc, int x, int y, IntPtr old);
}
