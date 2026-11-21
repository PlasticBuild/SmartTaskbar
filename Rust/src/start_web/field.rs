use super::{FIELD_PREFIX, pid, web_prefix};
use crate::policy::Rect;
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomation2, IUIAutomationValuePattern,
            UIA_EditControlTypeId, UIA_ValuePatternId,
        },
    },
    core::Interface,
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{GetForegroundWindow, PostMessageW},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Observation {
    pub source: usize,
    pub bounds: Option<Rect>,
    pub prefixed: bool,
}

pub(super) fn automation() -> Option<IUIAutomation> {
    unsafe {
        let client: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        if let Ok(settings) = client.cast::<IUIAutomation2>() {
            let _ = settings.SetConnectionTimeout(350);
            let _ = settings.SetTransactionTimeout(350);
        }
        Some(client)
    }
}

pub(super) fn read(client: &IUIAutomation, source_pid: u32) -> Option<(String, Option<Rect>)> {
    unsafe {
        let field = client.GetFocusedElement().ok()?;
        if field.CurrentProcessId().ok()? as u32 != source_pid
            || field.CurrentControlType().ok()? != UIA_EditControlTypeId
            || field.CurrentIsPassword().ok()?.as_bool()
        {
            return None;
        }
        let value: IUIAutomationValuePattern =
            field.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
        let value = value.CurrentValue().ok()?.to_string();
        let bounds = field
            .CurrentBoundingRectangle()
            .ok()
            .map(|r| Rect::new(r.left, r.top, r.right, r.bottom))
            .filter(|r| !r.is_empty());
        Some((value, bounds))
    }
}

pub(super) struct Watch {
    commands: mpsc::Sender<usize>,
    value: Arc<Mutex<Observation>>,
}

impl Watch {
    pub(super) fn start(hwnd: usize) -> Option<Self> {
        let (commands, requests) = mpsc::channel();
        let value = Arc::new(Mutex::new(Observation::default()));
        let result = value.clone();
        thread::Builder::new()
            .name("Start Search field".into())
            .spawn(move || {
                if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.is_err() {
                    return;
                }
                if let Some(client) = automation() {
                    observe(hwnd, &client, requests, &result);
                }
                unsafe { CoUninitialize() };
            })
            .ok()?;
        Some(Self { commands, value })
    }

    pub(super) fn select(&self, source: HWND) {
        let _ = self.commands.send(source as usize);
    }

    pub(super) fn current(&self, source: HWND) -> Observation {
        self.value
            .lock()
            .ok()
            .filter(|v| v.source == source as usize)
            .map_or_else(Observation::default, |v| *v)
    }
}

fn observe(
    hwnd: usize,
    client: &IUIAutomation,
    requests: mpsc::Receiver<usize>,
    result: &Mutex<Observation>,
) {
    let mut source = 0;
    loop {
        if source == 0 {
            let Ok(next) = requests.recv() else { break };
            source = next;
        }
        let source_pid = pid(source as HWND);
        let mut next = Observation {
            source,
            ..Default::default()
        };
        if source_pid != 0
            && pid(unsafe { GetForegroundWindow() }) == source_pid
            && let Some((value, bounds)) = read(client, source_pid)
        {
            next.bounds = bounds;
            next.prefixed = web_prefix(&value).is_some();
        }
        if let Ok(mut current) = result.lock()
            && *current != next
        {
            *current = next;
            unsafe {
                PostMessageW(
                    hwnd as HWND,
                    FIELD_PREFIX,
                    next.prefixed as usize,
                    source as isize,
                )
            };
        }
        match requests.recv_timeout(Duration::from_millis(85)) {
            Ok(next) => source = next,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}
