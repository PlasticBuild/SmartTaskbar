use crate::appearance::web_search::default_browser;
use serde_json::Value;
use std::{
    fmt::Write,
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
};
use windows_sys::Win32::Networking::WinHttp::{URL_COMPONENTS, WinHttpCrackUrl};

pub(super) struct Provider(String);

fn browser_data(browser: &Path) -> Option<PathBuf> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let product = browser.parent()?.parent()?.file_name()?.to_str()?;
    let data = match browser.file_name()?.to_str()?.to_ascii_lowercase().as_str() {
        "brave.exe"
            if matches!(
                product,
                "Brave-Browser" | "Brave-Origin" | "Brave-Browser-Beta" | "Brave-Browser-Nightly"
            ) =>
        {
            local.join("BraveSoftware").join(product)
        }
        "chrome.exe"
            if matches!(
                product,
                "Chrome" | "Chrome Beta" | "Chrome Dev" | "Chrome SxS"
            ) =>
        {
            local.join("Google").join(product)
        }
        "msedge.exe" if matches!(product, "Edge" | "Edge Beta" | "Edge Dev" | "Edge SxS") => {
            local.join("Microsoft").join(product)
        }
        "vivaldi.exe" => local.join("Vivaldi"),
        "chromium.exe" => local.join("Chromium"),
        _ => return None,
    };
    Some(data.join("User Data"))
}

fn json_file(path: &Path) -> Option<Value> {
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

impl Provider {
    pub fn current() -> Result<Self, &'static str> {
        let browser = default_browser().ok_or("Windows could not find your default browser.")?;
        let data = browser_data(&browser)
            .ok_or("Embedded search is unavailable for this default browser.")?;
        let local_state = json_file(&data.join("Local State")).unwrap_or_default();
        let profile = local_state["profile"]["last_used"]
            .as_str()
            .unwrap_or("Default");
        let mut components = Path::new(profile).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err("The default browser profile could not be identified.");
        }
        let preferences = json_file(&data.join(profile).join("Preferences"))
            .ok_or("The default browser’s search settings could not be read.")?;
        let settings = &preferences["default_search_provider_data"];
        let template = ["template_url_data", "mirrored_template_url_data"]
            .into_iter()
            .find_map(|key| settings[key].is_object().then_some(&settings[key]))
            .ok_or("The default browser has no readable web-search template.")?;
        if template["search_url_post_params"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
        {
            return Err(
                "This search engine requires a request type the embedded pane does not support.",
            );
        }
        let pattern = template["url"]
            .as_str()
            .ok_or("The default search engine has no web address.")?;
        let provider = Self(pattern.to_owned());
        provider.url("test")?;
        Ok(provider)
    }

    pub fn url(&self, query: &str) -> Result<String, &'static str> {
        let terms = query.trim();
        if terms.is_empty() {
            return Err("Enter a search query.");
        }
        let mut encoded = String::new();
        for byte in terms.bytes() {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                encoded.push(char::from(byte));
            } else {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
        let mut source = self.0.as_str();
        let mut output = String::new();
        let mut found_terms = false;
        let unsupported = "This search engine’s web address cannot be used in the embedded pane.";
        while let Some((prefix, tail)) = source.split_once('{') {
            let (parameter, remaining) = tail.split_once('}').ok_or(unsupported)?;
            output.push_str(prefix);
            match parameter {
                "searchTerms" | "searchTerms?" => {
                    found_terms = true;
                    output.push_str(&encoded);
                }
                "inputEncoding" | "outputEncoding" | "inputEncoding?" | "outputEncoding?" => {
                    output.push_str("UTF-8")
                }
                "google:baseURL" => output.push_str("https://www.google.com/"),
                key if key.ends_with('?') || key.starts_with("google:") => {}
                _ => return Err(unsupported),
            }
            source = remaining;
        }
        output.push_str(source);
        if !found_terms || !is_https(&output) {
            return Err(unsupported);
        }
        Ok(output)
    }
}

pub(super) fn is_https(url: &str) -> bool {
    if !url.starts_with("https://") || url.bytes().any(|b| b.is_ascii_control()) {
        return false;
    }
    let url = url.encode_utf16().chain([0]).collect::<Vec<_>>();
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        dwHostNameLength: u32::MAX,
        dwUserNameLength: u32::MAX,
        dwPasswordLength: u32::MAX,
        ..Default::default()
    };
    let ok = unsafe { WinHttpCrackUrl(url.as_ptr(), 0, 0, &mut parts) };
    ok != 0
        && parts.dwHostNameLength != 0
        && parts.dwUserNameLength == 0
        && parts.dwPasswordLength == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_template_uses_the_selected_engine_and_encodes_the_literal_query() {
        let provider =
            Provider("https://example.com/find?q={searchTerms}&e={inputEncoding}".into());
        assert_eq!(
            provider.url("café & tea").unwrap(),
            "https://example.com/find?q=caf%C3%A9%20%26%20tea&e=UTF-8"
        );
        assert!(
            Provider("https://user@example.com/?q={searchTerms}".into())
                .url("tea")
                .is_err()
        );
        assert!(
            Provider("file:///q={searchTerms}".into())
                .url("tea")
                .is_err()
        );
    }
}
