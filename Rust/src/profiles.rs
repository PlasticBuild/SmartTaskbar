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
type Geometry = Option<(i32, i32, i32)>;
const ISLAND: &str = include_str!("../../Sources/SmartTaskbar.Appearance/Profiles/Island.json");
const VERTICAL: &str =
    include_str!("../../Sources/SmartTaskbar.Appearance/Profiles/IslandVertical.json");
const FULL: &str = include_str!("../../Sources/SmartTaskbar.Appearance/Profiles/FullWidth.json");
const START: &str =
    "StartMenu.StartBlendedFlexFrame > Grid#FrameRoot > Grid#AnimationRoot > Grid#MainMenu";
const SEARCH: &str = "Cortana.UI.Views.TaskbarSearchPage > Grid#RootGrid";

fn rule<S: Into<String>>(
    profile: &mut Profile,
    target: &str,
    properties: impl IntoIterator<Item = S>,
) {
    let index = profile
        .keys()
        .filter(|key| key.ends_with("].target"))
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
            .replace("__TINT_OPACITY__", &opacity(s))
            .replace(
                " TintOpacity=",
                " TintLuminosityOpacity=\"0.64\" TintOpacity=",
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

fn taskbar(s: &Settings, joined: bool) -> Result<Profile, String> {
    let mut profile = template(if s.island { ISLAND } else { FULL }, s)?;
    if s.island {
        profile.extend(template(VERTICAL, s)?);
        profile.insert("clickThroughTaskbar".into(), "1".into());
        rotation_guard(&mut profile);
    }
    if s.animate && (s.hover_lift != 0 || s.click_depth != 0) {
        let hover = 1.0 + 0.035 * f64::from(s.hover_lift);
        let pressed = if s.click_depth == 0 {
            hover
        } else {
            1.0 - 0.03 * f64::from(s.click_depth)
        };
        let mut motion = vec!["RenderTransformOrigin=0.5,0.5".into()];
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
        rule(
            &mut profile,
            "Taskbar.TaskListButton > Grid#IconPanel@CommonStates > Image#Icon, Taskbar.TaskListButton > Taskbar.TaskListLabeledButtonPanel#IconPanel@CommonStates > Image#Icon",
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

fn alignment(s: &Settings) -> &'static str {
    match s.alignment {
        Alignment::Left => "Left",
        Alignment::Center => "Center",
        Alignment::Right => "Right",
    }
}

fn start_search(s: &Settings, geometry: Geometry) -> Profile {
    let mut profile = Profile::from([
        ("theme".into(), "".into()),
        ("disableNewStartMenuLayout".into(), "default".into()),
        ("webContentCustomJs".into(), "".into()),
    ]);
    let joined = geometry.is_some();
    rule(
        &mut profile,
        "Border#AcrylicBorder, Border#AppBorder, Border#AccentAppBorder",
        glass(s, joined),
    );
    rule(
        &mut profile,
        "Border#AcrylicOverlay, Border#LayerBorder, Border#AccentLayerBorder",
        ["Background=Transparent".into(), corner(s, joined)],
    );
    rule(
        &mut profile,
        "Border#StartDropShadow, Border#RootGridDropShadow, Border#DropShadow, Border#dropshadow",
        [corner(s, joined)],
    );
    if let Some((measured, _, bottom)) = geometry {
        // The caller accounts for narrow monitors when measuring Start. Search's native
        // host has a 600-DIP layout; keep wider islands from clipping its controls.
        let width = measured.min(START_MAX_DIP);
        rule(
            &mut profile,
            START,
            [format!("Width={width}"), format!("MaxWidth={width}")],
        );
        let mut search = vec![
            format!("Width={}", width.min(600)),
            format!("HorizontalAlignment={}", alignment(s)),
        ];
        if bottom > 0 {
            search.push(format!("Margin=0,0,0,{bottom}"))
        }
        rule(&mut profile, SEARCH, search);
        rule(
            &mut profile,
            &format!("{SEARCH} > Grid#OuterBorderGrid"),
            glass(s, true),
        );
    }
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

fn positions(s: &Settings, geometry: Geometry) -> Profile {
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
            geometry.map_or(0, |(_, shift, _)| shift).to_string(),
        );
    }
    let anchor = geometry.map_or("windowsDefault".into(), |_| {
        alignment(s).to_ascii_lowercase()
    });
    profile.insert("startMenu.horizontalAlignment".into(), anchor);
    profile.insert(
        "startMenu.verticalAlignment".into(),
        "windowsDefault".into(),
    );
    profile.insert("startMenu.verticalShift".into(), "0".into());
    profile.insert("startMenu.horizontalShift".into(), "0".into());
    profile
}

/// Geometry, when present, describes a measurable bottom island in DIPs: Start's
/// display-bounded width, the tray's signed horizontal shift, and Search's bottom inset.
/// Other edges keep Explorer's native flyout positions.
pub fn build(s: &Settings, geometry: Geometry) -> Result<Vec<Value>, String> {
    s.validate()?;
    let geometry = geometry.filter(|_| s.island);
    if geometry.is_some_and(|(width, shift, inset)| width <= 0 || shift > 0 || inset < 0) {
        return Err("Invalid bottom island geometry for the shell profiles.".into());
    }
    let joined = geometry.is_some();
    [
        taskbar(s, joined)?,
        start_search(s, geometry),
        notifications(s, joined),
        positions(s, geometry),
    ]
    .into_iter()
    .map(|map| serde_json::to_value(map).map_err(|err| err.to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn four_complete_profiles_have_contiguous_styles_and_no_tokens() {
        for s in [
            Settings::default(),
            Settings {
                island: false,
                ..Settings::default()
            },
        ] {
            for geometry in [None, Some((560, -160, 17))] {
                let profiles = build(&s, geometry).unwrap();
                assert_eq!(profiles.len(), IDS.len());
                for (i, profile) in profiles.iter().enumerate() {
                    assert!(!profile.to_string().contains("__"));
                    let map = profile.as_object().unwrap();
                    assert!(map.values().all(Value::is_string));
                    if i == 3 {
                        continue;
                    }
                    let count = map.keys().filter(|k| k.ends_with("].target")).count();
                    assert!(count > 0);
                    for n in 0..count {
                        let name = map[&format!("controlStyles[{n}].target")].as_str().unwrap();
                        let items = styles(profile, name);
                        assert!(!items.is_empty());
                        assert_eq!(
                            items.len(),
                            map.keys()
                                .filter(|k| k.starts_with(&format!("controlStyles[{n}].styles[")))
                                .count()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn bottom_width_shift_and_search_inset_are_available_before_open() {
        let s = Settings {
            alignment: Alignment::Right,
            ..Settings::default()
        };
        let result = build(&s, Some((568, -164, 28))).unwrap();
        assert_eq!(result[3]["notificationCenter.horizontalShift"], "-164");
        assert_eq!(result[3]["actionCenter.horizontalAlignment"], "right");
        assert_eq!(result[3]["startMenu.horizontalAlignment"], "right");
        assert_eq!(styles(&result[1], START), ["Width=568", "MaxWidth=568"]);
        assert_eq!(
            styles(&result[1], SEARCH),
            ["Width=568", "HorizontalAlignment=Right", "Margin=0,0,0,28"]
        );
        for panel in ["Grid#NotificationCenterGrid", "Grid#CalendarCenterGrid"] {
            assert!(styles(&result[2], panel).contains(&"Width=332".into()));
        }
        assert!(
            styles(&result[0], "Border#OverflowFlyoutBackgroundBorder")
                .last()
                .unwrap()
                .ends_with(",0,0")
        );
        let limit = build(&s, Some((3000, 0, 0))).unwrap();
        assert!(styles(&limit[1], START).contains(&format!("Width={START_MAX_DIP}")));
        assert!(styles(&limit[1], SEARCH).contains(&"Width=600".into()));
        assert!(
            styles(&build(&s, Some((320, 0, 0))).unwrap()[1], START).contains(&"Width=320".into())
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
        let result = build(&s, Some((600, -100, 12))).unwrap();
        assert_eq!(result[0]["clickThroughTaskbar"], "0");
        assert_eq!(
            result[3]["notificationCenter.horizontalAlignment"],
            "windowsDefault"
        );
        assert_eq!(result[3]["notificationCenter.horizontalShift"], "0");
        assert!(build(&Settings { radius: 200, ..s }, None).is_err());
        for invalid in [(0, 0, 0), (480, 1, 0), (480, 0, -1)] {
            assert!(build(&Settings::default(), Some(invalid)).is_err());
        }
    }
}
