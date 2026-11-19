using System.Drawing;
using SmartTaskbar;

var upper = new Rectangle(-200, -1440, 2560, 1440);
var lower = new Rectangle(0, 0, 1920, 1080);
var upperBar = new TaskbarInfo((IntPtr)10, (IntPtr)1, upper,
    new Rectangle(-200, -151, 2560, 151), TaskbarPosition.Bottom);
var lowerBar = new TaskbarInfo((IntPtr)20, (IntPtr)2, lower,
    new Rectangle(0, 1032, 1920, 48), TaskbarPosition.Bottom);

Expect((int)TaskbarPosition.Left == 0 && (int)TaskbarPosition.Top == 1 &&
    (int)TaskbarPosition.Right == 2 && (int)TaskbarPosition.Bottom == 3, "Win32 appbar edge mapping");

var hiddenUpper = new Rectangle(-200, -2, 2560, 151);
Expect(TaskbarInfo.Dock(hiddenUpper, upper, TaskbarPosition.Bottom) == upperBar.ShownBounds,
    "A hidden upper bar docks back inside the upper monitor");
Expect(!upperBar.IsShown(hiddenUpper), "A two-pixel strip counts as hidden even over a second display");
Expect(upperBar.VisibleThickness(hiddenUpper) == 2
    && upperBar.VisibleThickness(new Rectangle(-200, -12, 2560, 151)) == 12,
    "The exit wait can distinguish the hidden strip from a partly collapsed taskbar");
Expect(upperBar.IsShown(upperBar.ShownBounds), "A fully docked bar counts as shown");
Expect(!upperBar.IsShown(new Rectangle(2360, -151, 2560, 151)), "A bar on a different display is hidden here");

var band = upperBar.RevealBand();
Expect(band == new Rectangle(-200, -48, 2560, 48), "The reveal band covers the full taskbar edge");
Expect(band.Contains(400, -24) && !band.Contains(400, 0) && !band.Contains(400, -49),
    "Only the upper side of the shared edge triggers the upper bar");
Expect(lowerBar.RevealBand() == new Rectangle(0, 1064, 1920, 16),
    "The lower exterior edge gets its own reveal band");
Expect(band.Contains(-180, -24), "The unshared portion of an offset monitor can also reveal the bar");

var leftBar = new TaskbarInfo((IntPtr)30, (IntPtr)1, upper,
    new Rectangle(-200, -1440, 80, 1440), TaskbarPosition.Left);
Expect(leftBar.RevealBand() == new Rectangle(-200, -1440, 26, 1440),
    "A left-side taskbar can reveal from its outer edge");
var topBar = new TaskbarInfo((IntPtr)40, (IntPtr)2, lower, new Rectangle(0, 0, 1920, 48), TaskbarPosition.Top);
Expect(topBar.RevealBand() == new Rectangle(0, 0, 1920, 16),
    "A top-edge bar can reveal from the lower side of a seam");
var rightBar = new TaskbarInfo((IntPtr)50, (IntPtr)1, upper,
    new Rectangle(2280, -1440, 80, 1440), TaskbarPosition.Right);
Expect(rightBar.RevealBand() == new Rectangle(2334, -1440, 26, 1440),
    "A right-side taskbar can reveal from its outer edge");

var spanning = Rectangle.FromLTRB(350, -450, 1560, 1060);
Expect(upperBar.CoverageFrom(spanning, false, false) == TaskbarCoverage.Overlap
    && lowerBar.CoverageFrom(spanning, false, false) == TaskbarCoverage.Overlap, "A spanning window covers both bars");
Expect(upperBar.CoverageFrom(spanning, true, true) == TaskbarCoverage.Clear
    && lowerBar.CoverageFrom(lower, true, true) == TaskbarCoverage.Clear,
    "Work-area growth of an ordinary maximized window does not keep the taskbars hidden");
Expect(upperBar.CoverageFrom(Rectangle.FromLTRB(0, -250, 800, -151), false, false) == TaskbarCoverage.Clear,
    "Touching the taskbar boundary leaves the bar clear");
Expect(upperBar.CoverageFrom(upper, false, false) == TaskbarCoverage.Fullscreen
    && lowerBar.CoverageFrom(upper, false, false) == TaskbarCoverage.Clear,
    "Fullscreen is classified independently on each monitor");
Expect(lowerBar.CoverageFrom(lower, true, false) == TaskbarCoverage.Fullscreen,
    "A borderless maximized window that fills the actual display still counts as fullscreen");

var transition = new AutoHideTransition();
Expect(!transition.IsReady(false, true, 1000) && !transition.IsReady(false, true, 1124),
    "A short overlap does not start an auto-hide animation");
Expect(transition.IsReady(false, true, 1125), "A stable overlap can hide the taskbar");
Expect(transition.IsReady(false, false, 1130) && !transition.IsReady(false, true, 1135),
    "Clearing the overlap resets the transition timer");
Expect(!transition.IsReady(true, false, 1300) && transition.IsReady(true, false, 1500),
    "A clear desktop stays clear briefly before the taskbars repin");
Expect(!transition.IsReady(false, true, 2000, 400) && !transition.IsReady(false, true, 2399, 400)
    && transition.IsReady(false, true, 2400, 400), "Fullscreen gets time to settle before requesting Explorer auto-hide");

var dwell = new EdgeDwell();
Expect(!dwell.IsReady(upperBar.Handle, 1000) && !dwell.IsReady(upperBar.Handle, 1179),
    "Brief seam entry does not reveal a taskbar");
dwell = default;
Expect(!dwell.IsReady(upperBar.Handle, 1400) && dwell.IsReady(upperBar.Handle, 1580),
    "Reentering the seam starts a new dwell");
Expect(!dwell.IsReady(lowerBar.Handle, 1600) && dwell.IsReady(lowerBar.Handle, 1780),
    "Changing monitors restarts the dwell");
Expect(dwell.TryRequestReveal(1780) && !dwell.TryRequestReveal(2179),
    "An unconfirmed shell reveal gets time to animate before a retry");
Expect(dwell.TryRequestReveal(2180) && !dwell.TryRequestReveal(3000),
    "A stationary pointer can request at most two reveals");
Expect(!dwell.IsReady(upperBar.Handle, 3010) && dwell.TryRequestReveal(3190),
    "A new taskbar gets its own reveal attempts");

Console.WriteLine("29 taskbar geometry and transition checks passed.");

static void Expect(bool condition, string message)
{
    if (!condition)
        throw new InvalidOperationException(message);
}
