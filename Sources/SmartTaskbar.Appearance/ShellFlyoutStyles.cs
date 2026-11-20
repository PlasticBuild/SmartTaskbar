using System.Globalization;
using System.Text.Json;

namespace SmartTaskbar.Appearance;

internal static class ShellFlyoutStyles
{
    public const string StartStyler = "windows-11-start-menu-styler";
    public const string NotificationStyler = "windows-11-notification-center-styler";
    public const string Positions = "shell-flyout-positions";

    public static bool Matches(string? id, string? version) => id switch
    {
        StartStyler or NotificationStyler => version == "1.8",
        Positions => version == "1.3.1",
        _ => false
    };

    public static JsonElement Build(string id, AppearanceOptions options)
    {
        var glass = options.Layout is TaskbarLayout.FullWidth or TaskbarLayout.Island;
        if (id == Positions)
        {
            var island = options.Layout == TaskbarLayout.Island;
            return JsonSerializer.SerializeToElement(new Dictionary<string, string>
            {
                ["notificationCenter.horizontalAlignment"] = island ? "tray" : "windowsDefault",
                ["notificationCenter.horizontalShift"] = "0",
                ["actionCenter.horizontalAlignment"] = island ? "tray" : "windowsDefault",
                ["actionCenter.horizontalShift"] = "0",
                ["startMenu.horizontalAlignment"] = island ? "center" : "windowsDefault",
                ["startMenu.horizontalShift"] = "0",
                ["startMenu.verticalAlignment"] = "windowsDefault",
                ["startMenu.verticalShift"] = "0"
            });
        }

        var settings = new Dictionary<string, string> { ["theme"] = "" };
        if (id == StartStyler)
        {
            settings["disableNewStartMenuLayout"] = "default";
            settings["webContentCustomJs"] = "";
        }
        if (!glass)
            return JsonSerializer.SerializeToElement(settings);

        var panel = GlassPanel(options);
        switch (id)
        {
            case StartStyler:
                AddStyles(settings, "Border#AcrylicBorder, Border#AppBorder, Border#AccentAppBorder", panel);
                AddStyles(settings, "Border#AcrylicOverlay, Border#LayerBorder, Border#AccentLayerBorder",
                    ["Background=Transparent", SurfaceRadius(options)]);
                AddStyles(settings, "Border#StartDropShadow, Border#RootGridDropShadow, Border#DropShadow, Border#dropshadow",
                    [SurfaceRadius(options)]);
                AddContextMenus(settings, options);
                break;
            case NotificationStyler:
                AddStyles(settings, "Grid#NotificationCenterGrid, Grid#CalendarCenterGrid, Grid#ControlCenterRegion, " +
                    "Grid#MediaTransportControlsRegion, Border#JumpListRestyledAcrylic, Border#ToastBackgroundBorder, Border#ToastBackgroundBorder2",
                    panel);
                AddStyles(settings, "Grid#L1Grid > Border, Border#CalendarHeaderMinimizedOverlay, ScrollViewer#CalendarControlScrollViewer",
                    ["Background=Transparent"]);
                break;
        }
        return JsonSerializer.SerializeToElement(settings);
    }

    public static void AddTaskbarMenus(Dictionary<string, string> settings, AppearanceOptions options)
    {
        AddStyles(settings, "Border#OverflowFlyoutBackgroundBorder", GlassPanel(options));
        AddContextMenus(settings, options);
    }

    private static void AddContextMenus(Dictionary<string, string> settings, AppearanceOptions options)
    {
        AddStyles(settings, "MenuFlyoutPresenter", [SurfaceRadius(options)]);
        AddStyles(settings, "MenuFlyoutPresenter > Border, ToolTip > ContentPresenter#LayoutRoot", GlassPanel(options));
    }

    private static string SurfaceRadius(AppearanceOptions options)
        => FormattableString.Invariant($"CornerRadius={Math.Clamp(options.Radius + 6, 8, 24)}");

    private static string[] GlassPanel(AppearanceOptions options)
    {
        // Larger surfaces keep extra tint for legible text, while retaining the taskbar palette.
        var opacity = Math.Clamp(1 - options.Transparency / 125d, .38, 1).ToString("0.00", CultureInfo.InvariantCulture);
        return
        [
            $"Background:=<AcrylicBrush TintColor=\"{{ThemeResource SystemChromeMediumColor}}\" TintOpacity=\"{opacity}\" FallbackColor=\"{{ThemeResource SystemChromeLowColor}}\" />",
            "BorderBrush:=<SolidColorBrush Color=\"{ThemeResource SurfaceStrokeColorDefault}\" />",
            "BorderThickness=1",
            SurfaceRadius(options)
        ];
    }

    private static void AddStyles(Dictionary<string, string> settings, string target, string[] styles)
    {
        var index = settings.Keys.Count(key => key.StartsWith("controlStyles[", StringComparison.Ordinal) &&
            key.EndsWith(".target", StringComparison.Ordinal));
        var group = $"controlStyles[{index}]";
        settings[group + ".target"] = target;
        for (var style = 0; style < styles.Length; style++)
            settings[$"{group}.styles[{style}]"] = styles[style];
    }
}
