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
        if (options.Layout == TaskbarLayout.Island && id is StartStyler or Positions)
        {
            var geometry = PrimaryIslandWidth.Read();
            return id == Positions ? BuildPositions(options, geometry.TrayShift)
                : BuildStart(options, geometry.StartWidth, geometry.SearchInset);
        }
        return BuildCore(id, options, 0);
    }

    internal static bool IsCurrent(string id, AppearanceOptions options, JsonElement settings)
    {
        if (id == Positions && options.Layout == TaskbarLayout.Island)
        {
            int? shift = null;
            if (settings.TryGetProperty("notificationCenter.horizontalAlignment", out var mode) && mode.GetString() == "right" &&
                settings.TryGetProperty("notificationCenter.horizontalShift", out var value) &&
                int.TryParse(value.GetString(), CultureInfo.InvariantCulture, out var parsed) && parsed <= 0)
                shift = parsed;
            return JsonElement.DeepEquals(settings, BuildPositions(options, shift));
        }
        if (id != StartStyler || options.Layout != TaskbarLayout.Island)
            return JsonElement.DeepEquals(settings, Build(id, options));
        if (ReadStartWidth(settings) is not { } width) return false;
        var inset = 0;
        if (ReadRule(settings, SearchRoot, 2) is { } margin &&
            (!margin.StartsWith(SearchMargin, StringComparison.Ordinal) ||
                !int.TryParse(margin.AsSpan(SearchMargin.Length), out inset) || inset < 0))
            return false;
        // Exact comparison also recognizes the earlier profile, which used the native (zero) inset.
        return JsonElement.DeepEquals(settings, BuildStart(options, width, inset));
    }

    internal static int? ReadStartWidth(JsonElement settings)
    {
        var rule = ReadRule(settings, StartPanel, 0);
        return rule != null && rule.StartsWith("Width=", StringComparison.Ordinal) &&
            int.TryParse(rule.AsSpan(6), out var width) && width is > 0 and <= IslandMenuWidth.Maximum ? width : null;
    }

    private static string? ReadRule(JsonElement settings, string target, int index)
    {
        foreach (var property in settings.EnumerateObject())
        {
            if (!property.Name.EndsWith(".target", StringComparison.Ordinal) ||
                property.Value.GetString() != target)
                continue;
            var key = property.Name[..^7] + $".styles[{index}]";
            return settings.TryGetProperty(key, out var style) ? style.GetString() : null;
        }
        return null;
    }

    private const string StartPanel = "StartMenu.StartBlendedFlexFrame > Grid#FrameRoot > Grid#AnimationRoot > Grid#MainMenu";
    private const string SearchRoot = "Cortana.UI.Views.TaskbarSearchPage > Grid#RootGrid";
    private const string SearchMargin = "Margin=0,0,0,";

    internal static JsonElement BuildPositions(AppearanceOptions options, int? trayShift, bool legacy = false)
    {
        var island = options.Layout == TaskbarLayout.Island;
        var mode = island && trayShift.HasValue ? "right" : island && legacy ? "tray" : "windowsDefault";
        var offset = (trayShift ?? 0).ToString(CultureInfo.InvariantCulture);
        return JsonSerializer.SerializeToElement(new Dictionary<string, string>
        {
            ["notificationCenter.horizontalAlignment"] = mode,
            ["notificationCenter.horizontalShift"] = offset,
            ["actionCenter.horizontalAlignment"] = mode,
            ["actionCenter.horizontalShift"] = offset,
            ["startMenu.horizontalAlignment"] = island && (trayShift.HasValue || legacy)
                ? options.Alignment.ToString().ToLowerInvariant() : "windowsDefault",
            ["startMenu.horizontalShift"] = "0",
            ["startMenu.verticalAlignment"] = "windowsDefault",
            ["startMenu.verticalShift"] = "0"
        });
    }

    internal static JsonElement BuildCore(string id, AppearanceOptions options, int startWidth)
        => BuildContent(id, options, startWidth, 0);

    internal static JsonElement BuildStart(AppearanceOptions options, int startWidth, int searchInset)
        => BuildContent(StartStyler, options, startWidth, searchInset);

    private static JsonElement BuildContent(string id, AppearanceOptions options, int startWidth, int searchInset)
    {
        var glass = options.Layout is TaskbarLayout.FullWidth or TaskbarLayout.Island;
        if (id == Positions)
            return BuildPositions(options, null);

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
                    string[] search = ["Width=600", $"HorizontalAlignment={options.Alignment}"];
                    AddStyles(settings, SearchRoot, searchInset > 0
                        ? [.. search, SearchMargin + searchInset.ToString(CultureInfo.InvariantCulture)] : search);
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
