use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf, process::Command};
use windows_sys::Win32::UI::Shell::{ASSOCF_IS_PROTOCOL, ASSOCSTR_EXECUTABLE, AssocQueryStringW};

pub(crate) fn default_browser() -> Option<PathBuf> {
    let mut executable = vec![0; 32_768];
    let mut length = executable.len() as u32;
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL,
            ASSOCSTR_EXECUTABLE,
            windows_sys::w!("https"),
            windows_sys::w!("open"),
            executable.as_mut_ptr(),
            &mut length,
        )
    };
    if result != 0 {
        return None;
    }
    let end = executable.iter().position(|&unit| unit == 0)?;
    let path = PathBuf::from(OsString::from_wide(&executable[..end]));
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn search_argument(browser: &str, query: &str) -> Option<String> {
    match browser.to_ascii_lowercase().as_str() {
        "brave.exe" | "chrome.exe" | "chromium.exe" | "firefox.exe" | "msedge.exe" => {
            // The Windows search argument asks the browser to use its active search provider.
            Some(format!("? {query}"))
        }
        _ => None,
    }
}

pub(super) fn open(query: &str) -> Result<(), &'static str> {
    let browser = default_browser().ok_or("Windows could not find your default browser.")?;
    let argument = browser
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| search_argument(name, query))
        .ok_or("Direct search is unavailable for your default browser.")?;
    Command::new(browser)
        .arg(argument)
        .spawn()
        .map_err(|_| "Your default browser could not be opened.")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_browsers_receive_the_search_term_without_a_provider_url() {
        for browser in ["Brave.exe", "chrome.exe", "msedge.exe", "firefox.exe"] {
            assert_eq!(
                search_argument(browser, "-- café & tea \"cake\"").as_deref(),
                Some("? -- café & tea \"cake\"")
            );
        }
        assert_eq!(search_argument("custom.exe", "tea"), None);
    }
}
