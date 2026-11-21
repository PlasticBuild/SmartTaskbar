//! Windhawk settings for its four native shell styling components. The three shared
//! SmartTaskbar appearance templates are retained in their original location and covered
//! by the repository's MIT license (see ../../LICENSE). Shell control names are the
//! selectors exposed by the installed Windhawk stylers.
use crate::{
    policy::{SIDE_INSET_DIP, START_MAX_DIP},
    settings::{Alignment, Settings},
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const IDS: [&str; 4] = [
    "windows-11-taskbar-styler",
    "windows-11-start-menu-styler",
    "windows-11-notification-center-styler",
    "shell-flyout-positions",
];

type Profile = BTreeMap<String, String>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BottomIsland {
    pub start_width: i32,
    pub start_shift: i32,
    pub tray_shift: i32,
}
pub type Geometry = Option<BottomIsland>;
const ISLAND: &str = include_str!("../assets/taskbar/Island.json");
const VERTICAL: &str = include_str!("../assets/taskbar/IslandVertical.json");
const FULL: &str = include_str!("../assets/taskbar/FullWidth.json");
const START: &str =
    "StartMenu.StartBlendedFlexFrame > Grid#FrameRoot > Grid#AnimationRoot > Grid#MainMenu";
fn rule<S: Into<String>>(
    profile: &mut Profile,
    target: &str,
    properties: impl IntoIterator<Item = S>,
) {
    let index = profile
        .keys()
        .filter(|key| key.starts_with("controlStyles[") && key.ends_with("].target"))
        .count();
    let group = format!("controlStyles[{index}]");
    profile.insert(format!("{group}.target"), target.into());
    for (n, value) in properties.into_iter().enumerate() {
        profile.insert(format!("{group}.styles[{n}]"), value.into());
    }
}

fn opacity(s: &Settings) -> String {
    format!("{:.2}", 1.0 - f64::from(s.transparency) / 100.0)
}

fn taskbar_brush(s: &Settings) -> String {
    format!(
        "<SolidColorBrush Color=\"{{ThemeResource SystemChromeMediumColor}}\" Opacity=\"{}\" />",
        opacity(s)
    )
}

fn corner(s: &Settings, joined: bool) -> String {
    let r = (s.radius + 6).clamp(8, 24);
    if joined {
        format!("CornerRadius={r},{r},0,0")
    } else {
        format!("CornerRadius={r}")
    }
}

fn glass(s: &Settings, joined: bool) -> Vec<String> {
    vec![
        format!(
            "Background:=<AcrylicBrush TintColor=\"{{ThemeResource SystemChromeMediumColor}}\" TintOpacity=\"{}\" TintLuminosityOpacity=\"0.64\" FallbackColor=\"{{ThemeResource SystemChromeLowColor}}\" />",
            opacity(s)
        ),
        "BorderBrush:=<SolidColorBrush Color=\"{ThemeResource SurfaceStrokeColorDefault}\" />"
            .into(),
        "BorderThickness=1".into(),
        corner(s, joined),
    ]
}

fn style_context_menus(profile: &mut Profile, s: &Settings) {
    rule(profile, "MenuFlyoutPresenter", [corner(s, false)]);
    rule(
        profile,
        "MenuFlyoutPresenter > Border, ToolTip > ContentPresenter#LayoutRoot",
        glass(s, false),
    );
}

fn template(text: &str, s: &Settings) -> Result<Profile, String> {
    let side = SIDE_INSET_DIP.to_string();
    let mut values: Profile =
        serde_json::from_str(text).map_err(|err| format!("Invalid appearance template: {err}"))?;
    for val in values.values_mut() {
        *val = val
            .replace(
                "__LEADING__",
                if s.alignment == Alignment::Left {
                    &side
                } else {
                    "*"
                },
            )
            .replace(
                "__TRAILING__",
                if s.alignment == Alignment::Right {
                    &side
                } else {
                    "*"
                },
            )
            .replace(
                "__EMBEDDED_INSET__",
                if s.alignment == Alignment::Center {
                    "max((BarWidth-$Width)/2,0)"
                } else {
                    "0"
                },
            )
            .replace("__RADIUS__", &s.radius.to_string())
            .replace("__TRANSPARENCY__", &s.transparency.to_string())
            .replace(
                "<AcrylicBrush TintColor=\"{ThemeResource SystemChromeMediumColor}\" TintOpacity=\"__TINT_OPACITY__\" FallbackColor=\"{ThemeResource SystemChromeLowColor}\" />",
                &taskbar_brush(s),
            );
        if val.contains("__") {
            return Err(format!("Unresolved appearance value: {val}"));
        }
    }
    Ok(values)
}

fn rotation_guard(profile: &mut Profile) {
    // At rotation the XAML host can become tall one layout pass before DockingStates
    // changes. Never use the old horizontal width as the new taskbar's thickness.
    let values = [
        (
            "styleConstants[2]",
            "Horizontal=(DockEdge==`left`?0:(DockEdge==`right`?0:(HostWidth==``?1:(HostHeight==``?1:HostWidth>=HostHeight))))",
        ),
        ("controlStyles[2].styles[1]", "Tag@DockedLeft=left"),
        ("controlStyles[2].styles[2]", "Tag@DockedRight=right"),
        (
            "controlStyles[1].styles[3]",
            "MaxWidth={{$Horizontal?skip():(NativeButtonWidth==``?skip():(NativeButtonWidth>0?NativeButtonWidth:skip()))}}",
        ),
        (
            "controlStyles[1].styles[4]",
            "HorizontalAlignment={{DockEdge==`right`?`Right`:(DockEdge==`left`?`Left`:skip())}}",
        ),
    ];
    profile.extend(values.map(|(k, v)| (k.into(), v.into())));
    rule(
        profile,
        "Taskbar.ExperienceToggleButton[AutomationProperties.AutomationId=StartButton]",
        ["ActualWidth=>NativeButtonWidth"],
    );
    rule(
        profile,
        ":root > ScrollViewer",
        ["ActualWidth=>HostWidth", "ActualHeight=>HostHeight"],
    );
}

fn taskbar(s: &Settings, joined: bool, reserve: crate::media::Reserve) -> Result<Profile, String> {
    let centered;
    let s = if reserve.horizontal != 0 && s.island {
        centered = Settings {
            alignment: Alignment::Center,
            ..s.clone()
        };
        &centered
    } else {
        s
    };
    let mut profile = template(if s.island { ISLAND } else { FULL }, s)?;
    if s.island {
        profile.extend(template(VERTICAL, s)?);
        profile.insert("clickThroughTaskbar".into(), "1".into());
        rotation_guard(&mut profile);
        if reserve != crate::media::Reserve::default() {
            let left = reserve.horizontal.max(0);
            let right = (-reserve.horizontal).max(0);
            let top = reserve.vertical.max(0);
            let bottom = (-reserve.vertical).max(0);
            profile.insert(
                "controlStyles[0].styles[4]".into(),
                format!("Margin={{{{$Horizontal?{left}:0}}}},{{{{$Horizontal?0:{top}}}}},{{{{$Horizontal?{right}:0}}}},{{{{$Horizontal?0:{bottom}}}}}"),
            );
            if reserve.horizontal != 0 {
                profile.insert(
                    "styleConstants[6]".into(),
                    format!("Inset=max((BarWidth-{}-$Width)/2,0)", left + right),
                );
            }
            if reserve.vertical != 0 {
                profile.insert(
                    "styleConstants[10]".into(),
                    format!(
                        "VerticalInset=max((BarHeight-{}-$IslandHeight)/2,0)",
                        top + bottom
                    ),
                );
            }
            let radius = s.radius;
            let leading_x = if reserve.horizontal > 0 { 0 } else { radius };
            let trailing_x = if reserve.horizontal < 0 { 0 } else { radius };
            let leading_y = if reserve.vertical > 0 { 0 } else { radius };
            let trailing_y = if reserve.vertical < 0 { 0 } else { radius };
            profile.insert("controlStyles[8].styles[3]".into(), format!(
                "CornerRadius={{{{$Horizontal?($Embedded?`{leading_x},{trailing_x},{trailing_x},{leading_x}`:`{leading_x},0,0,{leading_x}`):($VerticalEmbedded?`{leading_y},{leading_y},{trailing_y},{trailing_y}`:`{leading_y},{leading_y},0,0`)}}}}"
            ));
            profile.insert("controlStyles[11].styles[3]".into(), format!(
                "CornerRadius={{{{$Horizontal?($Embedded?skip():`0,{trailing_x},{trailing_x},0`):($VerticalEmbedded?skip():`0,0,{trailing_y},{trailing_y}`)}}}}"
            ));
        }
    }
    let animate_icons = s.animate && (s.hover_lift != 0 || s.click_depth != 0);
    {
        // Set the image dimensions in DIPs because the shell's default changes with orientation.
        let size = f64::from(s.icon_size) * 24.0 / 100.0;
        let mut motion = vec![
            format!("Width={size:.2}"),
            format!("Height={size:.2}"),
            format!("MaxWidth={size:.2}"),
            format!("MaxHeight={size:.2}"),
            format!("MinWidth={size:.2}"),
            format!("MinHeight={size:.2}"),
            "Stretch=Uniform".into(),
        ];
        if animate_icons {
            motion.push("RenderTransformOrigin=0.5,0.5".into());
            let hover = 1.0 + 0.035 * f64::from(s.hover_lift);
            let pressed = if s.click_depth == 0 {
                hover
            } else {
                1.0 - 0.03 * f64::from(s.click_depth)
            };
            for group in [
                "Inactive",
                "Active",
                "MultiWindow",
                "RequestingAttention",
                "RequestingAttentionMulti",
            ] {
                for (state, scale, y) in [
                    ("PointerOver", hover, -i16::from(s.hover_lift)),
                    (
                        "Pressed",
                        pressed,
                        if s.click_depth == 0 {
                            -i16::from(s.hover_lift)
                        } else {
                            i16::from(s.click_depth)
                        },
                    ),
                ] {
                    motion.push(format!("RenderTransform@{group}{state}:=<CompositeTransform ScaleX=\"{scale:.3}\" ScaleY=\"{scale:.3}\" TranslateY=\"{y}\" />"));
                }
            }
        }
        rule(
            &mut profile,
            "Taskbar.TaskListButton > Grid#IconPanel@CommonStates > Image#Icon, Taskbar.TaskListButton > Taskbar.TaskListLabeledButtonPanel#IconPanel@CommonStates > Image#Icon, Taskbar.TaskListButton > Taskbar.TaskListButtonPanel#IconPanel@CommonStates > Image#Icon",
            motion,
        );
    }
    rule(
        &mut profile,
        "Border#OverflowFlyoutBackgroundBorder",
        glass(s, joined),
    );
    style_context_menus(&mut profile, s);
    Ok(profile)
}

fn start_menu(s: &Settings, geometry: Geometry) -> Profile {
    let mut profile = Profile::from([
        ("theme".into(), "".into()),
        ("disableNewStartMenuLayout".into(), "default".into()),
    ]);
    let joined = geometry.is_some();
    rule(&mut profile, "Border#AcrylicBorder", glass(s, joined));
    rule(
        &mut profile,
        "Border#AcrylicOverlay",
        ["Background=Transparent".into(), corner(s, joined)],
    );
    let radius = s.radius;
    let frame = [
        format!("Background:={}", taskbar_brush(s)),
        "Visibility=Visible".into(),
        "BorderBrush:=<SolidColorBrush Color=\"{ThemeResource SurfaceStrokeColorDefault}\" />"
            .into(),
        if joined {
            format!("CornerRadius={radius},{radius},0,0")
        } else {
            format!("CornerRadius={radius}")
        },
        "BorderThickness=1".into(),
    ];
    rule(
        &mut profile,
        "Border#StartDropShadow, Border#RootGridDropShadow, Border#DropShadow, Border#dropshadow",
        [corner(s, joined)],
    );
    if let Some(island) = geometry {
        let width = island.start_width.min(START_MAX_DIP);
        rule(
            &mut profile,
            START,
            [format!("Width={width}"), format!("MaxWidth={width}")],
        );
    }
    rule(&mut profile, "Border#AppBorder", frame);
    style_context_menus(&mut profile, s);
    profile
}

fn notifications(s: &Settings, joined: bool) -> Profile {
    let mut profile = Profile::from([("theme".into(), "".into())]);
    rule(
        &mut profile,
        "Grid#MediaTransportControlsRegion, Border#ToastBackgroundBorder, Border#ToastBackgroundBorder2",
        glass(s, false),
    );
    rule(
        &mut profile,
        "Border#JumpListRestyledAcrylic",
        glass(s, joined),
    );
    if joined {
        rule(
            &mut profile,
            "ActionCenter.NotificationCenterPage > Grid#RootGrid > Grid#RootContent",
            ["RowSpacing=0"],
        );
        let mut notify = glass(s, true);
        notify.extend(
            [
                "Width=332",
                "HorizontalAlignment=Right",
                "VerticalAlignment=Bottom",
                "Margin=0,0,0,-3",
                "MinHeight=0",
                "Shadow:=",
            ]
            .map(String::from),
        );
        rule(&mut profile, "Grid#NotificationCenterGrid", notify);
        let mut calendar = glass(s, false);
        calendar.pop();
        calendar.extend(
            [
                "CornerRadius=0",
                "Width=332",
                "HorizontalAlignment=Right",
                "Margin=0,-2,0,12",
                "Shadow:=",
            ]
            .map(String::from),
        );
        rule(&mut profile, "Grid#CalendarCenterGrid", calendar);
        let mut quick = glass(s, true);
        quick.extend(["Width=344", "HorizontalAlignment=Right"].map(String::from));
        rule(&mut profile, "Grid#ControlCenterRegion", quick);
    } else {
        rule(
            &mut profile,
            "Grid#NotificationCenterGrid, Grid#CalendarCenterGrid, Grid#ControlCenterRegion",
            glass(s, false),
        );
    }
    rule(
        &mut profile,
        "Grid#L1Grid > Border, Border#CalendarHeaderMinimizedOverlay, ScrollViewer#CalendarControlScrollViewer",
        ["Background=Transparent"],
    );
    profile
}

fn positions(geometry: Geometry, start: Geometry) -> Profile {
    let mut profile = Profile::new();
    for panel in ["actionCenter", "notificationCenter"] {
        profile.insert(
            format!("{panel}.horizontalAlignment"),
            if geometry.is_some() {
                "right"
            } else {
                "windowsDefault"
            }
            .into(),
        );
        profile.insert(
            format!("{panel}.horizontalShift"),
            geometry.map_or(0, |island| island.tray_shift).to_string(),
        );
    }
    profile.insert(
        "startMenu.horizontalAlignment".into(),
        if start.is_some() {
            "center"
        } else {
            "windowsDefault"
        }
        .into(),
    );
    profile.insert(
        "startMenu.verticalAlignment".into(),
        "windowsDefault".into(),
    );
    profile.insert("startMenu.verticalShift".into(), "0".into());
    profile.insert(
        "startMenu.horizontalShift".into(),
        start.map_or(0, |island| island.start_shift).to_string(),
    );
    profile
}

/// Geometry describes the measured bottom island in DIPs for Start and tray panels.
/// Other edges keep Explorer's native flyout positions.
pub fn build(s: &Settings, geometry: Geometry) -> Result<Vec<Value>, String> {
    build_with_reserve(s, geometry, crate::media::Reserve::default())
}

pub fn build_with_reserve(
    s: &Settings,
    geometry: Geometry,
    reserve: crate::media::Reserve,
) -> Result<Vec<Value>, String> {
    s.validate()?;
    let geometry = geometry.filter(|_| s.island);
    if geometry.is_some_and(|island| island.start_width <= 0 || island.tray_shift > 0) {
        return Err("Invalid bottom island geometry for the shell profiles.".into());
    }
    let joined = geometry.is_some();
    let start_geometry = geometry.filter(|_| s.match_start_width);
    [
        taskbar(s, joined, reserve)?,
        start_menu(s, start_geometry),
        notifications(s, joined),
        positions(geometry, start_geometry),
    ]
    .into_iter()
    .map(|map| serde_json::to_value(map).map_err(|err| err.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(start_width: i32, tray_shift: i32) -> Geometry {
        Some(BottomIsland {
            start_width,
            start_shift: -110,
            tray_shift,
        })
    }

    fn styles(profile: &Value, selector: &str) -> Vec<String> {
        let fields = profile.as_object().unwrap();
        let group = fields
            .iter()
            .find_map(|(key, val)| {
                (val.as_str() == Some(selector))
                    .then(|| key.strip_suffix(".target"))
                    .flatten()
            })
            .expect("missing shell selector");
        (0..)
            .map_while(|n| {
                fields
                    .get(&format!("{group}.styles[{n}]"))
                    .map(|v| v.as_str().unwrap().to_owned())
            })
            .collect()
    }

    #[test]
    fn app_icon_size_applies_in_both_layouts_and_with_motion_disabled() {
        for island in [false, true] {
            let settings = Settings {
                island,
                icon_size: 80,
                hover_lift: 5,
                animate: false,
                ..Settings::default()
            };
            let reduced = taskbar(&settings, false, Default::default()).unwrap();
            assert!(reduced.values().any(|value| value == "Width=19.20"));
            assert!(reduced.values().any(|value| value == "Height=19.20"));
            assert!(!reduced.values().any(|v| v.starts_with("RenderTransform@")));
            let motion = taskbar(
                &Settings {
                    animate: true,
                    ..settings
                },
                false,
                Default::default(),
            )
            .unwrap();
            assert!(
                motion
                    .values()
                    .any(|v| v.starts_with("RenderTransform@ActivePointerOver")
                        && v.contains("ScaleX=\"1.175\""))
            );
        }
    }

    #[test]
    fn taskbar_transparency_controls_actual_brush_opacity() {
        for island in [false, true] {
            for (transparency, opacity) in [(0, "1.00"), (85, "0.15")] {
                let settings = Settings {
                    island,
                    transparency,
                    ..Settings::default()
                };
                let profile = taskbar(&settings, false, Default::default()).unwrap();
                let index = if island { 8 } else { 0 };
                let brush = &profile[&format!("controlStyles[{index}].styles[0]")];
                assert!(brush.contains("<SolidColorBrush "));
                assert!(brush.contains(&format!("Opacity=\"{opacity}\"")));
                if island {
                    assert!(
                        profile["controlStyles[11].styles[0]"]
                            .contains(&format!("Opacity=\"{opacity}\""))
                    );
                }
            }
        }
    }

    #[test]
    fn four_complete_profiles_have_contiguous_styles_and_no_tokens() {
        for s in [
            Settings::default(),
            Settings {
                island: false,
                ..Settings::default()
            },
        ] {
            for geometry in [None, geometry(560, -160)] {
                let profiles = build(&s, geometry).unwrap();
                assert_eq!(profiles.len(), IDS.len());
                for (i, profile) in profiles.iter().enumerate() {
                    assert!(!profile.to_string().contains("__"));
                    let map = profile.as_object().unwrap();
                    assert!(map.values().all(Value::is_string));
                    assert!(
                        map.values()
                            .all(|v| !v.as_str().unwrap().contains(['\r', '\n']))
                    );
                    if i == 3 {
                        continue;
                    }
                    for collection in ["controlStyles", "webContentStyles"] {
                        let count = map
                            .keys()
                            .filter(|k| k.starts_with(collection) && k.ends_with("].target"))
                            .count();
                        assert_eq!(count > 0, collection == "controlStyles");
                        for n in 0..count {
                            let key = format!("{collection}[{n}]");
                            let name = map[&format!("{key}.target")].as_str().unwrap();
                            let items = styles(profile, name);
                            assert!(!items.is_empty());
                            assert_eq!(
                                items.len(),
                                map.keys()
                                    .filter(|k| k.starts_with(&format!("{key}.styles[")))
                                    .count()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn start_menu_keeps_the_native_glass_in_each_layout() {
        let settings = Settings {
            radius: 11,
            transparency: 73,
            match_start_width: true,
            ..Settings::default()
        };
        for geometry in [None, geometry(545, -27)] {
            let profile = build(&settings, geometry).unwrap().remove(1);
            let background = styles(&profile, "Border#AppBorder");
            assert_eq!(
                background[0],
                format!("Background:={}", taskbar_brush(&settings))
            );
            assert!(background[0].contains("Opacity=\"0.27\""));
            assert_eq!(
                background[3],
                if geometry.is_some() {
                    "CornerRadius=11,11,0,0"
                } else {
                    "CornerRadius=11"
                }
            );
        }
    }

    #[test]
    fn width_matching_is_opt_in_for_bottom_island_start() {
        let island = geometry(568, -164);
        let native = build(&Settings::default(), island).unwrap();
        assert_eq!(native[3]["startMenu.horizontalAlignment"], "windowsDefault");
        assert_eq!(native[3]["startMenu.horizontalShift"], "0");
        assert_eq!(native[3]["notificationCenter.horizontalShift"], "-164");
        assert!(
            !native[1]
                .as_object()
                .unwrap()
                .values()
                .any(|value| value == START)
        );
        let frame = styles(&native[1], "Border#AppBorder");
        assert!(frame.iter().any(|entry| entry.starts_with("Background:=")));
        assert!(frame.contains(&"CornerRadius=4".into()));
        assert!(!frame.iter().any(|entry| entry.contains("Width=")));

        let mut settings = Settings {
            island: false,
            match_start_width: true,
            ..Settings::default()
        };
        let full = build(&settings, island).unwrap();
        assert_eq!(full, build(&settings, None).unwrap());
        settings.island = true;
        assert_eq!(build(&settings, None).unwrap()[1], full[1]);
    }

    #[test]
    fn bottom_start_width_and_shift_are_available_before_open() {
        let s = Settings {
            alignment: Alignment::Right,
            match_start_width: true,
            ..Settings::default()
        };
        let result = build(&s, geometry(568, -164)).unwrap();
        assert_eq!(result[3]["notificationCenter.horizontalShift"], "-164");
        assert_eq!(result[3]["actionCenter.horizontalAlignment"], "right");
        assert_eq!(result[3]["startMenu.horizontalAlignment"], "center");
        assert_eq!(result[3]["startMenu.horizontalShift"], "-110");
        assert_eq!(styles(&result[1], START), ["Width=568", "MaxWidth=568"]);
        for panel in ["Grid#NotificationCenterGrid", "Grid#CalendarCenterGrid"] {
            assert!(styles(&result[2], panel).contains(&"Width=332".into()));
        }
        assert!(
            styles(&result[0], "Border#OverflowFlyoutBackgroundBorder")
                .last()
                .unwrap()
                .ends_with(",0,0")
        );
        let limit = build(&s, geometry(3000, 0)).unwrap();
        assert!(styles(&limit[1], START).contains(&format!("Width={START_MAX_DIP}")));
        assert!(
            styles(&build(&s, geometry(320, 0)).unwrap()[1], START).contains(&"Width=320".into())
        );
    }

    #[test]
    fn rotation_and_appearance_changes_preserve_native_controls() {
        let s = Settings {
            radius: 12,
            hover_lift: 4,
            click_depth: 3,
            alignment: Alignment::Left,
            transparency: 20,
            ..Settings::default()
        };
        let result = build(&s, None).unwrap();
        let taskbar = &result[0];
        assert_eq!(taskbar["clickThroughTaskbar"], "1");
        assert_eq!(
            taskbar["controlStyles[2].styles[2]"],
            "Tag@DockedRight=right"
        );
        assert!(
            taskbar["styleConstants[2]"]
                .as_str()
                .unwrap()
                .contains("HostWidth>=HostHeight")
        );
        let layout = taskbar["controlStyles[0].styles[0]"].as_str().unwrap();
        assert!(layout.contains("Width=\"12\""));
        assert!(
            taskbar
                .as_object()
                .unwrap()
                .values()
                .any(|v| v.as_str().unwrap().contains("TranslateY=\"-4\""))
        );
        assert!(
            taskbar
                .as_object()
                .unwrap()
                .values()
                .any(|v| v.as_str().unwrap().contains("TranslateY=\"3\""))
        );
        let overflow = styles(taskbar, "Border#OverflowFlyoutBackgroundBorder");
        assert!(overflow[0].contains("TintOpacity=\"0.80\""));
        assert_eq!(overflow[3], "CornerRadius=18");
        assert_eq!(result[3]["startMenu.horizontalAlignment"], "windowsDefault");
        assert!(!result[1].as_object().unwrap().values().any(|v| v == START));
        let reduced = build(
            &Settings {
                animate: false,
                ..s
            },
            None,
        )
        .unwrap();
        assert!(!reduced[0].to_string().contains("RenderTransform@"));
    }

    #[test]
    fn full_width_preserves_windows_positions_and_invalid_inputs_are_rejected() {
        let s = Settings {
            island: false,
            ..Settings::default()
        };
        let result = build(&s, geometry(600, -100)).unwrap();
        assert_eq!(result[0]["clickThroughTaskbar"], "0");
        assert_eq!(
            result[3]["notificationCenter.horizontalAlignment"],
            "windowsDefault"
        );
        assert_eq!(result[3]["notificationCenter.horizontalShift"], "0");
        assert!(build(&Settings { radius: 200, ..s }, None).is_err());
        for (width, shift) in [(0, 0), (480, 1)] {
            assert!(build(&Settings::default(), geometry(width, shift)).is_err());
        }
    }
}
