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
        => BuildCore(id, options, id == StartStyler && options.Layout == TaskbarLayout.Island ? PrimaryIslandWidth.Read() : 0);

    internal static bool IsCurrent(string id, AppearanceOptions options, JsonElement settings)
    {
        if (id != StartStyler || options.Layout != TaskbarLayout.Island)
            return JsonElement.DeepEquals(settings, Build(id, options));
        foreach (var property in settings.EnumerateObject())
        {
            if (!property.Name.EndsWith(".target", StringComparison.Ordinal) ||
                property.Value.GetString() != StartPanel)
                continue;
            var key = property.Name[..^7] + ".styles[0]";
            if (settings.TryGetProperty(key, out var style) && style.GetString() is { } rule &&
                rule.StartsWith("Width=", StringComparison.Ordinal) && int.TryParse(rule.AsSpan(6), out var width) &&
                width is > 0 and <= IslandMenuWidth.Maximum)
                return JsonElement.DeepEquals(settings, BuildCore(id, options, width));
        }
        return false;
    }

    private const string StartPanel = "StartMenu.StartBlendedFlexFrame > Grid#FrameRoot > Grid#AnimationRoot > Grid#MainMenu";

    internal static JsonElement BuildCore(string id, AppearanceOptions options, int startWidth)
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
                ["startMenu.horizontalAlignment"] = island ? options.Alignment.ToString().ToLowerInvariant() : "windowsDefault",
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

        var islandLayout = options.Layout == TaskbarLayout.Island;
        var panel = GlassPanel(options);
        switch (id)
        {
            case StartStyler:
                AddStyles(settings, "Border#AcrylicBorder, Border#AppBorder, Border#AccentAppBorder",
                    islandLayout ? DockedPanel(options) : panel);
                AddStyles(settings, "Border#AcrylicOverlay, Border#LayerBorder, Border#AccentLayerBorder",
                    ["Background=Transparent", SurfaceRadius(options, islandLayout)]);
                AddStyles(settings, "Border#StartDropShadow, Border#RootGridDropShadow, Border#DropShadow, Border#dropshadow",
                    [SurfaceRadius(options, islandLayout)]);
                if (islandLayout)
                {
                    AddStyles(settings, StartPanel, [$"Width={startWidth}", $"MaxWidth={startWidth}"]);
                    AddStyles(settings, "Cortana.UI.Views.TaskbarSearchPage > Grid#RootGrid",
                        ["Width=600", "HorizontalAlignment=Center"]);
                    AddStyles(settings, "Cortana.UI.Views.TaskbarSearchPage > Grid#RootGrid > Grid#OuterBorderGrid",
                        DockedPanel(options));
                }
                AddContextMenus(settings, options);
                break;
            case NotificationStyler:
                AddStyles(settings, "Grid#MediaTransportControlsRegion, Border#ToastBackgroundBorder, Border#ToastBackgroundBorder2",
                    panel);
                AddStyles(settings, "Border#JumpListRestyledAcrylic", islandLayout ? DockedPanel(options) : panel);
                if (islandLayout)
                {
                    AddStyles(settings, "ActionCenter.NotificationCenterPage > Grid#RootGrid > Grid#RootContent",
                        ["RowSpacing=0"]);
                    AddStyles(settings, "Grid#NotificationCenterGrid",
                        [.. DockedPanel(options), "Width=332", "HorizontalAlignment=Right", "VerticalAlignment=Bottom",
                            "Margin=0,0,0,-3", "MinHeight=0", "Shadow:="]);
                    AddStyles(settings, "Grid#CalendarCenterGrid",
                        [.. panel.Take(3), "CornerRadius=0", "Width=332", "HorizontalAlignment=Right",
                            "Margin=0,-2,0,12", "Shadow:="]);
                    AddStyles(settings, "Grid#ControlCenterRegion",
                        [.. DockedPanel(options), "Width=344", "HorizontalAlignment=Right"]);
                }
                else
                    AddStyles(settings, "Grid#NotificationCenterGrid, Grid#CalendarCenterGrid, Grid#ControlCenterRegion", panel);
                AddStyles(settings, "Grid#L1Grid > Border, Border#CalendarHeaderMinimizedOverlay, ScrollViewer#CalendarControlScrollViewer",
                    ["Background=Transparent"]);
                break;
        }
        return JsonSerializer.SerializeToElement(settings);
    }

    public static void AddTaskbarMenus(Dictionary<string, string> settings, AppearanceOptions options, bool joined = true)
    {
        if (joined)
        {
            foreach (var key in settings.Keys.ToArray())
                if (settings[key].Contains("<AcrylicBrush ", StringComparison.Ordinal))
                    settings[key] = settings[key].Replace(" TintOpacity=", " TintLuminosityOpacity=\"0.64\" TintOpacity=",
                        StringComparison.Ordinal);
        }
        AddStyles(settings, "Border#OverflowFlyoutBackgroundBorder", joined && options.Layout == TaskbarLayout.Island
            ? DockedPanel(options) : GlassPanel(options, joined));
        AddContextMenus(settings, options, joined);
    }

    private static void AddContextMenus(Dictionary<string, string> settings, AppearanceOptions options, bool joined = true)
    {
        AddStyles(settings, "MenuFlyoutPresenter", [SurfaceRadius(options)]);
        AddStyles(settings, "MenuFlyoutPresenter > Border, ToolTip > ContentPresenter#LayoutRoot", GlassPanel(options, joined));
    }

    private static string SurfaceRadius(AppearanceOptions options, bool docked = false)
    {
        var radius = Math.Clamp(options.Radius + 6, 8, 24);
        return docked ? FormattableString.Invariant($"CornerRadius={radius},{radius},0,0")
            : FormattableString.Invariant($"CornerRadius={radius}");
    }

    private static string[] DockedPanel(AppearanceOptions options)
        => [..GlassPanel(options).Take(3), SurfaceRadius(options, true)];

    private static string[] GlassPanel(AppearanceOptions options, bool joined = true)
    {
        var opacity = joined ? 1 - options.Transparency / 100d : Math.Clamp(1 - options.Transparency / 125d, .38, 1);
        var glass = joined ? " TintLuminosityOpacity=\"0.64\"" : "";
        return
        [
            $"Background:=<AcrylicBrush TintColor=\"{{ThemeResource SystemChromeMediumColor}}\" TintOpacity=\"{opacity.ToString("0.00", CultureInfo.InvariantCulture)}\"{glass} FallbackColor=\"{{ThemeResource SystemChromeLowColor}}\" />",
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
