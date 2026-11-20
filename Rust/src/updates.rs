use serde::Deserialize;
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
};
use windows_sys::Win32::Networking::WinHttp::*;

pub const REPOSITORY: &str = "https://github.com/PlasticBuild/SmartTaskbar";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    New(String),
    Current,
    NoRelease,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn version(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let value = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(value)
}

fn interpret(data: &[u8], current: &str) -> Result<Status, String> {
    let release: Release =
        serde_json::from_slice(data).map_err(|_| "GitHub returned an unreadable release.")?;
    if release.draft || release.prerelease {
        return Ok(Status::NoRelease);
    }
    let latest = version(&release.tag_name)
        .ok_or("The latest GitHub release has an unsupported version tag.")?;
    let installed = version(current).ok_or("This build has an unsupported version.")?;
    if latest > installed {
        Ok(Status::New(format!(
            "{}.{}.{}",
            latest[0], latest[1], latest[2]
        )))
    } else {
        Ok(Status::Current)
    }
}

struct Http(*mut c_void);
impl Http {
    fn new(handle: *mut c_void) -> Result<Self, String> {
        if handle.is_null() {
            Err("Could not start an HTTPS request to GitHub.".into())
        } else {
            Ok(Self(handle))
        }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn check() -> Result<Status, String> {
    unsafe {
        let session = Http::new(WinHttpOpen(
            wide(concat!("SmartTaskbar/", env!("CARGO_PKG_VERSION"))).as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ))?;
        WinHttpSetTimeouts(session.0, 3500, 3500, 3500, 5000);
        let host = Http::new(WinHttpConnect(
            session.0,
            wide("api.github.com").as_ptr(),
            443,
            0,
        ))?;
        let request = Http::new(WinHttpOpenRequest(
            host.0,
            null(),
            wide("/repos/PlasticBuild/SmartTaskbar/releases/latest").as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ))?;
        let headers = wide("Accept: application/vnd.github+json\r\n");
        if WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            (headers.len() - 1) as u32,
            null(),
            0,
            0,
            0,
        ) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err("GitHub could not be reached. Try again in Updates.".into());
        }
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        ) == 0
        {
            return Err("GitHub did not return a release status.".into());
        }
        match status {
            200 => {}
            404 => return Ok(Status::NoRelease),
            403 | 429 => return Err("GitHub is limiting update checks. Try again later.".into()),
            _ => return Err(format!("GitHub returned HTTP {status}.")),
        }
        let mut body = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let mut count = 0;
            if WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut count,
            ) == 0
            {
                return Err("The release response could not be read.".into());
            }
            if count == 0 {
                return interpret(&body, CURRENT);
            }
            if body.len() + count as usize > 512 * 1024 {
                return Err("The release response exceeded the size limit.".into());
            }
            body.extend_from_slice(&buffer[..count as usize]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_newer_stable_releases_are_announced() {
        let decode = |tag, prerelease, draft| {
            let body =
                serde_json::json!({ "tag_name": tag, "prerelease": prerelease, "draft": draft });
            interpret(&serde_json::to_vec(&body).unwrap(), "0.2.9")
        };
        assert_eq!(
            decode("v0.2.10", false, false),
            Ok(Status::New("0.2.10".into()))
        );
        assert_eq!(decode("v0.2.9", false, false), Ok(Status::Current));
        assert_eq!(decode("v0.2.0", false, false), Ok(Status::Current));
        assert_eq!(decode("v0.3.0-beta.1", true, false), Ok(Status::NoRelease));
        assert_eq!(decode("v1.0.0", false, true), Ok(Status::NoRelease));
        assert!(decode("nightly", false, false).is_err());
    }
}
