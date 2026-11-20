//! Import appearance values from a saved taskbar styler export without reading the desktop.
use crate::settings::{Alignment, Settings};
use serde_json::{Map, Value};

const FRAME: &str = "Taskbar.TaskbarFrame > Grid#RootGrid > Taskbar.TaskbarBackground > Grid";
const ROOT: &str = ":root > ScrollViewer > ScrollContentPresenter > Border";
const DOCK: &str = "Taskbar.TaskbarFrame > Grid#RootGrid@DockingStates";
const TRAY: &str =
    "Taskbar.TaskbarExtensionElement > ContentPresenter > SystemTray.SystemTrayFrame";

fn rules<'a>(profile: &'a Map<String, Value>, target: &str) -> Vec<&'a str> {
    let group = profile.iter().find_map(|(key, value)| {
        let group = key.strip_suffix(".target")?;
        (group.starts_with("controlStyles[") && value.as_str() == Some(target)).then_some(group)
    });
    let Some(group) = group else {
        return Vec::new();
    };
    let prefix = format!("{group}.styles[");
    profile
        .iter()
        .filter_map(|(key, value)| key.starts_with(&prefix).then(|| value.as_str()).flatten())
        .collect()
}

pub fn from_export(value: &Value) -> Result<Settings, String> {
    let profile = value
        .pointer("/data/settings")
        .unwrap_or(value)
        .as_object()
        .ok_or("Expected a taskbar appearance settings object.")?;
    let frame = rules(profile, FRAME);
    let base = frame.iter().copied().find(|s| {
        s.starts_with("Background:=<AcrylicBrush ")
            && s.contains("TintColor=\"{ThemeResource SystemChromeMediumColor}\"")
    });
    let is_glass = base.is_some()
        && profile.get("theme").and_then(Value::as_str) == Some("")
        && profile
            .get("xamlDiagnosticsHandling")
            .and_then(Value::as_str)
            == Some("alert")
        && matches!(
            profile.get("clickThroughTaskbar").and_then(Value::as_str),
            Some("0" | "1")
        )
        && ["BackgroundFill", "BackgroundStroke"].iter().all(|part| {
            rules(profile, &format!("{FRAME} > Rectangle#{part}")).contains(&"Fill=Transparent")
        });
    let dock = rules(profile, DOCK);
    let island = rules(profile, ROOT).contains(&"ActualWidth=>BarWidth")
        && dock.contains(&"Tag=>DockEdge")
        && rules(profile, TRAY).contains(&"ActualWidth=>EmbeddedTrayWidth");
    let full = frame.contains(&"BorderThickness=0,1,0,0") && dock.is_empty();
    if !is_glass || island == full {
        return Err("The export does not match a managed SmartTaskbar full-width or floating-island profile.".into());
    }
    let mut result = Settings {
        island,
        ..Settings::default()
    };
    // The earliest presets expressed these values directly in XAML. Later presets
    // include the user's original numbers in named constants regardless of style indices.
    if let Some(value) = base
        .and_then(|s| s.split_once(" TintOpacity=\""))
        .and_then(|(_, s)| s.split('"').next())
    {
        let opacity: f64 = value
            .parse()
            .map_err(|_| "Invalid acrylic opacity in the appearance export.")?;
        if !(0.0..=1.0).contains(&opacity) {
            return Err("Invalid acrylic opacity in the appearance export.".into());
        }
        result.transparency = ((1.0 - opacity) * 100.0).round() as u8;
    }
    if let Some(radius) = frame.iter().find_map(|r| r.strip_prefix("CornerRadius=")) {
        let numeric = radius
            .strip_prefix("{{$Horizontal?($Embedded?\x60")
            .and_then(|r| r.split('\x60').next())
            .unwrap_or_else(|| radius.split(',').next().unwrap_or(""));
        if let Ok(radius) = numeric.parse() {
            result.radius = radius;
        }
    }
    let mut seen = std::collections::HashSet::new();
    for (key, val) in profile {
        if !key.starts_with("styleConstants[") {
            continue;
        }
        let Some((name, value)) = val.as_str().and_then(|v| v.split_once('=')) else {
            continue;
        };
        if !matches!(
            name,
            "SmartTaskbarRadius"
                | "SmartTaskbarTransparency"
                | "SmartTaskbarHoverLift"
                | "SmartTaskbarClickDepth"
                | "SmartTaskbarAlignment"
        ) {
            continue;
        }
        if !seen.insert(name) {
            return Err(format!("Duplicate {name} in the appearance export."));
        }
        let n = value
            .parse::<u8>()
            .map_err(|_| format!("Invalid {name} in the appearance export."))?;
        match name {
            "SmartTaskbarRadius" => result.radius = n,
            "SmartTaskbarTransparency" => result.transparency = n,
            "SmartTaskbarHoverLift" => result.hover_lift = n,
            "SmartTaskbarClickDepth" => result.click_depth = n,
            "SmartTaskbarAlignment" => {
                result.alignment = match n {
                    0 => Alignment::Center,
                    1 => Alignment::Left,
                    2 => Alignment::Right,
                    _ => return Err("Unknown island alignment in the appearance export.".into()),
                }
            }
            _ => unreachable!(),
        }
    }
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn example(island: bool) -> Value {
        let mut f = json!({"theme":"","clickThroughTaskbar":"0","xamlDiagnosticsHandling":"alert",
            "controlStyles[1].target":FRAME,
            "controlStyles[1].styles[0]":"Background:=<AcrylicBrush TintColor=\"{ThemeResource SystemChromeMediumColor}\" TintOpacity=\"0.63\" />",
            "controlStyles[2].target":format!("{FRAME} > Rectangle#BackgroundFill"), "controlStyles[2].styles[0]":"Fill=Transparent",
            "controlStyles[3].target":format!("{FRAME} > Rectangle#BackgroundStroke"), "controlStyles[3].styles[0]":"Fill=Transparent"});
        if island {
            for (i, (target, rule)) in [
                (ROOT, "ActualWidth=>BarWidth"),
                (TRAY, "ActualWidth=>EmbeddedTrayWidth"),
                (DOCK, "Tag=>DockEdge"),
            ]
            .into_iter()
            .enumerate()
            {
                f[format!("controlStyles[{}].target", i + 4)] = json!(target);
                f[format!("controlStyles[{}].styles[0]", i + 4)] = json!(rule);
            }
        } else {
            f["controlStyles[1].styles[1]"] = json!("BorderThickness=0,1,0,0");
        }
        f
    }
    #[test]
    fn named_appearance_preferences_override_the_rendered_brush() {
        let mut data = example(true);
        for (i, v) in [
            "SmartTaskbarRadius=11",
            "SmartTaskbarTransparency=70",
            "SmartTaskbarHoverLift=6",
            "SmartTaskbarClickDepth=3",
            "SmartTaskbarAlignment=2",
        ]
        .iter()
        .enumerate()
        {
            data[format!("styleConstants[{i}]")] = json!(v);
        }
        data["StartAtSignIn"] = json!(1);
        data["SearchRedirect"] = json!(true);
        let s = from_export(&json!({"data":{"settings":data}})).unwrap();
        assert!(s.island);
        assert_eq!(
            (s.radius, s.transparency, s.hover_lift, s.click_depth),
            (11, 70, 6, 3)
        );
        assert_eq!(s.alignment, Alignment::Right);
        assert!(!s.start_at_sign_in && !s.search_redirect);
    }
    #[test]
    fn older_full_width_presets_keep_defaults_for_unexported_properties() {
        let full = from_export(&example(false)).unwrap();
        assert!(!full.island);
        assert_eq!(full.transparency, 37);
        assert_eq!(full.radius, Settings::default().radius);
        assert_eq!(full.hover_lift, Settings::default().hover_lift);
    }
    #[test]
    fn rejects_flyouts_foreign_selectors_duplicate_and_out_of_range_values() {
        let mut f = example(true);
        f["controlStyles[2].styles[0]"] = json!("Fill=Red");
        assert!(from_export(&f).is_err());
        assert!(from_export(&json!({"theme":""})).is_err());
        let mut f = example(false);
        f["styleConstants[0]"] = json!("SmartTaskbarRadius=40");
        assert!(from_export(&f).is_err());
        f["styleConstants[1]"] = json!("SmartTaskbarRadius=4");
        assert!(from_export(&f).unwrap_err().contains("Duplicate"));
    }
}
