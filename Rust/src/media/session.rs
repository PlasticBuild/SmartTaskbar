use super::{Action, Art, Metadata, Timeline, Track};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    Foundation::TypedEventHandler,
    Graphics::Imaging::*,
    Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackInfo as Playback,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
    },
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{PostThreadMessageW, WM_NULL},
};

type Latest = Arc<Mutex<Option<Arc<Track>>>>;
enum Event {
    Changed,
    Metadata,
    Control(u64, Action),
    Stop,
}

pub struct Worker {
    tx: SyncSender<Event>,
    latest: Latest,
    stopping: Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    pub fn start() -> Option<Self> {
        let (tx, rx) = mpsc::sync_channel(32);
        let latest = Latest::default();
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_id = unsafe { GetCurrentThreadId() };
        let (sender, output, stop) = (tx.clone(), latest.clone(), stopping.clone());
        thread::Builder::new()
            .name("taskbar media".into())
            .spawn(move || {
                if unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok() {
                    listen(sender, rx, output, thread_id, &stop);
                    unsafe { RoUninitialize() };
                }
            })
            .ok()?;
        Some(Self {
            tx,
            latest,
            stopping,
        })
    }

    pub fn snapshot(&self) -> Option<Arc<Track>> {
        self.latest.lock().ok()?.clone()
    }

    pub fn controller(&self) -> impl Fn(u64, Action) + 'static + use<> {
        let tx = self.tx.clone();
        move |generation, action| {
            let _ = tx.try_send(Event::Control(generation, action));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.tx.try_send(Event::Stop);
    }
}

struct Sessions {
    manager: Manager,
    current: Option<i64>,
    changed: Option<i64>,
    playback: Vec<(Session, Option<i64>)>,
    active: Option<Active>,
    serial: u64,
}

struct Active {
    session: Session,
    metadata_event: Option<i64>,
    timeline_event: Option<i64>,
    details: Arc<Metadata>,
    paused_at: Option<Instant>,
}

impl Active {
    fn new(session: Session, tx: &SyncSender<Event>) -> Self {
        let sender = tx.clone();
        let metadata_event = session
            .MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = sender.try_send(Event::Metadata);
                Ok(())
            }))
            .ok();
        let sender = tx.clone();
        let timeline_event = session
            .TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = sender.try_send(Event::Changed);
                Ok(())
            }))
            .ok();
        let details = Arc::new(read_details(&session));
        Self {
            session,
            metadata_event,
            timeline_event,
            details,
            paused_at: None,
        }
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        if let Some(token) = self.metadata_event {
            let _ = self.session.RemoveMediaPropertiesChanged(token);
        }
        if let Some(token) = self.timeline_event {
            let _ = self.session.RemoveTimelinePropertiesChanged(token);
        }
    }
}

impl Sessions {
    fn new(manager: Manager, tx: &SyncSender<Event>) -> Self {
        let sender = tx.clone();
        let current = manager
            .CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                let _ = sender.try_send(Event::Changed);
                Ok(())
            }))
            .ok();
        let sender = tx.clone();
        let changed = manager
            .SessionsChanged(&TypedEventHandler::new(move |_, _| {
                let _ = sender.try_send(Event::Changed);
                Ok(())
            }))
            .ok();
        Self {
            manager,
            current,
            changed,
            playback: Vec::new(),
            active: None,
            serial: 0,
        }
    }

    fn update(&mut self, tx: &SyncSender<Event>, metadata_changed: bool) -> Option<Track> {
        let sessions: Vec<_> = self.manager.GetSessions().ok()?.into_iter().collect();
        self.playback.retain(|(session, token)| {
            let present = sessions.contains(session);
            if !present && let Some(token) = token {
                let _ = session.RemovePlaybackInfoChanged(*token);
            }
            present
        });
        for session in &sessions {
            if !self.playback.iter().any(|(watched, _)| watched == session) {
                let sender = tx.clone();
                let token = session
                    .PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                        let _ = sender.try_send(Event::Changed);
                        Ok(())
                    }))
                    .ok();
                self.playback.push((session.clone(), token));
            }
        }

        let playing = |session: &Session| {
            session
                .GetPlaybackInfo()
                .ok()
                .filter(|p| p.PlaybackStatus().ok() == Some(Status::Playing))
                .map(|p| (session.clone(), p))
        };
        let current = self.manager.GetCurrentSession().ok();
        let selected = current
            .as_ref()
            .and_then(playing)
            .or_else(|| {
                self.active
                    .as_ref()
                    .filter(|a| sessions.contains(&a.session))
                    .and_then(|a| playing(&a.session))
            })
            .or_else(|| sessions.iter().find_map(playing))
            .or_else(|| {
                let active = self
                    .active
                    .as_ref()
                    .filter(|a| sessions.contains(&a.session))?;
                let info = active.session.GetPlaybackInfo().ok()?;
                (info.PlaybackStatus().ok() == Some(Status::Paused))
                    .then(|| (active.session.clone(), info))
            });
        let Some((session, info)) = selected else {
            self.active = None;
            return None;
        };
        if self.active.as_ref().is_none_or(|a| a.session != session) {
            self.serial = self.serial.wrapping_add(1);
            self.active = Some(Active::new(session, tx));
        } else if metadata_changed && let Some(active) = self.active.as_mut() {
            self.serial = self.serial.wrapping_add(1);
            active.details = Arc::new(read_details(&active.session));
        }

        let active = self.active.as_mut()?;
        let playing = info.PlaybackStatus().ok() == Some(Status::Playing);
        if playing {
            active.paused_at = None;
        } else if active.paused_at.get_or_insert_with(Instant::now).elapsed()
            >= Duration::from_secs(8)
        {
            self.active = None;
            return None;
        }
        let controls = info.Controls().ok();
        let allowed = |get: fn(
            &windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls,
        ) -> windows::core::Result<bool>| {
            controls.as_ref().is_some_and(|c| get(c).unwrap_or(false))
        };
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls as Controls;
        Some(Track {
            generation: self.serial,
            details: active.details.clone(),
            playing,
            previous: allowed(Controls::IsPreviousEnabled),
            toggle: allowed(Controls::IsPlayPauseToggleEnabled)
                || if playing {
                    allowed(Controls::IsPauseEnabled)
                } else {
                    allowed(Controls::IsPlayEnabled)
                },
            next: allowed(Controls::IsNextEnabled),
            can_seek: allowed(Controls::IsPlaybackPositionEnabled),
            timeline: timeline(&active.session, &info, playing),
        })
    }

    fn control(&self, generation: u64, action: Action) {
        let Some(active) = self.active.as_ref().filter(|_| generation == self.serial) else {
            return;
        };
        let s = &active.session;
        let command = match action {
            Action::Previous => s.TrySkipPreviousAsync(),
            Action::Next => s.TrySkipNextAsync(),
            Action::Seek(ticks) => s.TryChangePlaybackPositionAsync(ticks),
            Action::PlayPause => {
                let info = s.GetPlaybackInfo().ok();
                if info
                    .as_ref()
                    .and_then(|p| p.Controls().ok())
                    .is_some_and(|c| c.IsPlayPauseToggleEnabled().unwrap_or(false))
                {
                    s.TryTogglePlayPauseAsync()
                } else if info.and_then(|p| p.PlaybackStatus().ok()) == Some(Status::Playing) {
                    s.TryPauseAsync()
                } else {
                    s.TryPlayAsync()
                }
            }
        };
        if let Ok(command) = command {
            let _ = command.join();
        }
    }
}

impl Drop for Sessions {
    fn drop(&mut self) {
        if let Some(token) = self.current {
            let _ = self.manager.RemoveCurrentSessionChanged(token);
        }
        if let Some(token) = self.changed {
            let _ = self.manager.RemoveSessionsChanged(token);
        }
        for (session, token) in &self.playback {
            if let Some(token) = token {
                let _ = session.RemovePlaybackInfoChanged(*token);
            }
        }
    }
}

fn listen(
    tx: SyncSender<Event>,
    rx: Receiver<Event>,
    latest: Latest,
    thread: u32,
    stopping: &std::sync::atomic::AtomicBool,
) {
    let manager = loop {
        if stopping.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if let Ok(manager) = Manager::RequestAsync().and_then(|a| a.join()) {
            break manager;
        }
        if matches!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected)
        ) {
            return;
        }
    };
    let mut sessions = Sessions::new(manager, &tx);
    let mut metadata = false;
    while !stopping.load(std::sync::atomic::Ordering::Relaxed) {
        let track = sessions.update(&tx, metadata).map(Arc::new);
        if let Ok(mut view) = latest.lock() {
            let redraw = view.is_some() || track.is_some();
            *view = track;
            if redraw {
                unsafe { PostThreadMessageW(thread, WM_NULL, 0, 0) };
            }
        }
        metadata = false;
        // Events wake immediately. The slower fallback covers providers that never emit changes.
        let first = match rx.recv_timeout(Duration::from_secs(2)) {
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            result => result.ok(),
        };
        for event in first.into_iter().chain(rx.try_iter().take(32)) {
            match event {
                Event::Stop => return,
                Event::Metadata => metadata = true,
                Event::Control(generation, action)
                    if !metadata && !stopping.load(std::sync::atomic::Ordering::Relaxed) =>
                {
                    sessions.control(generation, action)
                }
                Event::Control(..) => {}
                Event::Changed => {}
            }
        }
    }
}

fn timeline(session: &Session, info: &Playback, playing: bool) -> Option<Timeline> {
    let timeline = session.GetTimelineProperties().ok()?;
    let start = timeline.StartTime().ok()?.Duration.max(0);
    let end = timeline.EndTime().ok()?.Duration;
    let first = timeline
        .MinSeekTime()
        .map_or(start, |t| t.Duration.max(start));
    let seek_end = timeline.MaxSeekTime().map_or(end, |t| t.Duration);
    let last = if seek_end > first && end > first {
        seek_end.min(end)
    } else {
        seek_end.max(end)
    };
    if last <= first {
        return None;
    }
    let sampled = Instant::now();
    let rate = if playing {
        info.PlaybackRate()
            .and_then(|r| r.Value())
            .ok()
            .filter(|r| r.is_finite())
            .unwrap_or(1.0)
            .clamp(-16.0, 16.0)
    } else {
        0.0
    };
    let position = timeline.Position().ok()?.Duration;
    let now_ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        / 100
        + 116_444_736_000_000_000;
    let last_update = timeline.LastUpdatedTime().ok()?.UniversalTime;
    let past = (now_ticks as i64).saturating_sub(last_update).max(0);
    let position = if last_update > 0 {
        position.saturating_add((past as f64 * rate) as i64)
    } else {
        position
    };
    Some(Timeline {
        first,
        last,
        position,
        sampled,
        rate,
    })
}

fn read_details(session: &Session) -> Metadata {
    let Ok(item) = session.TryGetMediaPropertiesAsync().and_then(|a| a.join()) else {
        return Metadata::default();
    };
    Metadata {
        title: item
            .Title()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default(),
        artist: item
            .Artist()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default(),
        art: item
            .Thumbnail()
            .ok()
            .and_then(|thumbnail| decode_art(&thumbnail).ok())
            .map(blur_backdrop),
    }
}

fn blur_backdrop(mut art: Art) -> Art {
    let (w, h) = (art.width as usize, art.height as usize);
    let radius = (w.min(h) / 6).clamp(1, 15);
    let mut horizontal = art.pixels.clone();
    for y in 0..h {
        for x in 0..w {
            let left = x.saturating_sub(radius);
            let right = (x + radius).min(w - 1);
            let index = (y * w + x) * 4;
            for channel in 0..3 {
                let sum: usize = (left..=right)
                    .map(|column| usize::from(art.pixels[(y * w + column) * 4 + channel]))
                    .sum();
                horizontal[index + channel] = (sum / (right - left + 1)) as u8;
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let top = y.saturating_sub(radius);
            let bottom = (y + radius).min(h - 1);
            let index = (y * w + x) * 4;
            for (channel, tint) in [22, 17, 14].into_iter().enumerate() {
                let sum: usize = (top..=bottom)
                    .map(|row| usize::from(horizontal[(row * w + x) * 4 + channel]))
                    .sum();
                let color = sum / (bottom - top + 1);
                // A dark wash preserves white control contrast across bright artwork.
                art.pixels[index + channel] = ((color * 45 + tint * 55) / 100) as u8;
            }
        }
    }
    art
}

fn decode_art(
    thumb: &windows::Storage::Streams::IRandomAccessStreamReference,
) -> windows::core::Result<Art> {
    let stream = thumb.OpenReadAsync()?.join()?;
    if stream.Size()? > 16 * 1024 * 1024 {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070057u32 as i32,
        )));
    }
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (source_width, source_height) =
        (decoder.PixelWidth()?.max(1), decoder.PixelHeight()?.max(1));
    let longest = u64::from(source_width.max(source_height));
    let scale = |axis: u32| (u64::from(axis) * longest.min(96) / longest).max(1) as u32;
    let (width, height) = (scale(source_width), scale(source_height));
    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(width)?;
    transform.SetScaledHeight(height)?;
    let pixels = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Ignore,
            &transform,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?
        .DetachPixelData()?
        .to_vec();
    if pixels.len() != width as usize * height as usize * 4 {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070057u32 as i32,
        )));
    }
    Ok(Art {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
        core::Interface,
    };

    #[test]
    #[ignore = "requires the Windows media session broker"]
    fn subscribes_to_the_real_windows_media_broker() {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let manager = Manager::RequestAsync().unwrap().join().unwrap();
        let (tx, _) = mpsc::sync_channel(32);
        let sessions = Sessions::new(manager, &tx);
        assert!(sessions.current.is_some() && sessions.changed.is_some());
        sessions.manager.GetSessions().unwrap();
        drop(sessions);
        unsafe { RoUninitialize() };
    }

    #[test]
    fn windows_decoder_loads_bgra_cover_art_from_the_session_stream() {
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let stream = InMemoryRandomAccessStream::new().unwrap();
        let writer = DataWriter::CreateDataWriter(&stream.GetOutputStreamAt(0).unwrap()).unwrap();
        // A single 24-bit BMP pixel with a padded row.
        let mut bmp = vec![0u8; 58];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&1u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1u32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp[54..57].copy_from_slice(&[0x21, 0x53, 0xb4]);
        writer.WriteBytes(&bmp).unwrap();
        assert_eq!(writer.StoreAsync().unwrap().join().unwrap(), 58);
        writer.DetachStream().unwrap();
        stream.Seek(0).unwrap();
        let thumb = RandomAccessStreamReference::CreateFromStream(&stream).unwrap();
        let art = decode_art(&thumb.cast().unwrap()).unwrap();
        assert_eq!((art.width, art.height), (1, 1));
        assert_eq!(&art.pixels[..3], &[0x21, 0x53, 0xb4]);
        drop((thumb, writer, stream));
        unsafe { RoUninitialize() };
    }
}
#[test]
fn blur_uses_nearby_artwork_and_darkens_bright_pixels() {
    let art = blur_backdrop(Art {
        width: 3,
        height: 1,
        pixels: vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255],
    });
    assert!(art.pixels[0] > 20 && art.pixels[0] < 100);
    assert!(art.pixels[4] > 20 && art.pixels[4] < 100);
    assert_eq!(art.pixels[0], art.pixels[8]);
}
