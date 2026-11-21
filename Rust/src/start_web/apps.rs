use std::mem::size_of;
use webview2_com::take_pwstr;
use windows::{
    Win32::{
        System::Com::{CLSCTX_ALL, CoCreateInstance},
        UI::Shell::{
            AO_NONE, ApplicationActivationManager, BHID_EnumItems, IApplicationActivationManager,
            IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SHGetIDListFromObject,
            SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        },
    },
    core::{HSTRING, PCWSTR},
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::Com::CoTaskMemFree,
    UI::{
        Shell::{SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};

pub(super) struct App {
    pub name: String,
    normalized: String,
    item: IShellItem,
}

pub(super) fn installed() -> Vec<App> {
    unsafe {
        let Ok(folder): Result<IShellItem, _> =
            SHCreateItemFromParsingName(windows::core::w!("shell:AppsFolder"), None)
        else {
            return Vec::new();
        };
        let Ok(items): Result<IEnumShellItems, _> = folder.BindToHandler(None, &BHID_EnumItems)
        else {
            return Vec::new();
        };
        let mut apps = Vec::new();
        let mut next = [None];
        while items.Next(&mut next, None).is_ok() {
            let Some(item) = next[0].take() else { break };
            if let Ok(name) = item.GetDisplayName(SIGDN_NORMALDISPLAY) {
                let name = take_pwstr(name);
                if !name.is_empty() {
                    apps.push(App {
                        normalized: name.to_lowercase(),
                        name,
                        item,
                    });
                }
            }
        }
        apps.sort_by(|a, b| a.normalized.cmp(&b.normalized));
        apps
    }
}

pub(super) fn filter(apps: &[App], query: &str) -> Vec<usize> {
    matching(apps.iter().map(|app| app.normalized.as_str()), query)
}

fn matching<'a>(names: impl IntoIterator<Item = &'a str>, query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut matches = names
        .into_iter()
        .enumerate()
        .filter_map(|(index, name)| {
            if query.split_whitespace().all(|word| name.contains(word)) {
                Some((!name.starts_with(&query), index))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    matches.sort_unstable();
    matches.into_iter().map(|(_, index)| index).collect()
}

pub(super) fn launch(app: &App, owner: HWND) -> bool {
    unsafe {
        if let Ok(name) = app.item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING) {
            let name = take_pwstr(name);
            if name.contains('!')
                && let Ok(activation) = CoCreateInstance::<_, IApplicationActivationManager>(
                    &ApplicationActivationManager,
                    None,
                    CLSCTX_ALL,
                )
                && activation
                    .ActivateApplication(&HSTRING::from(name), PCWSTR::null(), AO_NONE)
                    .is_ok()
            {
                return true;
            }
        }
        let Ok(id) = SHGetIDListFromObject(&app.item) else {
            return false;
        };
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_NOASYNC,
            hwnd: owner,
            nShow: SW_SHOWNORMAL,
            lpIDList: id.cast(),
            ..Default::default()
        };
        let success = ShellExecuteExW(&mut info) != 0;
        CoTaskMemFree(id.cast());
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_app_search_matches_words_and_ranks_the_app_prefix_first() {
        let names = ["windows calculator", "calculator help", "weather"];
        assert_eq!(matching(names, "CALCULATOR"), [1, 0]);
        assert_eq!(matching(names, "calculator windows"), [0]);
    }
}
