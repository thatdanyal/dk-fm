//! Windows' media overlay and media keys (System Media Transport Controls): the song, its cover
//! and the position in the volume / media flyout and on the lock screen, and play / pause / next /
//! previous / seek from keyboards, headsets and the flyout. Uses the `windows` crate DK.FM already
//! has (souvlaki, used on macOS and Linux, brought an older copy of it).
#![cfg(windows)]
use crate::player::Player;
use std::sync::Arc;
use windows::core::HSTRING;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls, SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsDisplayUpdater,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::StorageFile;
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;

pub struct Controls {
    smtc: SystemMediaTransportControls,
    display: SystemMediaTransportControlsDisplayUpdater,
    timeline: SystemMediaTransportControlsTimelineProperties,
}

/// Seconds as a WinRT TimeSpan (100 ns ticks); anything odd (NaN, negative) is 0.
fn span(secs: f64) -> TimeSpan {
    let s = if secs.is_finite() { secs.clamp(0.0, 1e7) } else { 0.0 };
    TimeSpan { Duration: (s * 1e7) as i64 }
}

impl Controls {
    pub fn new(hwnd: isize, player: Arc<Player>) -> windows::core::Result<Self> {
        let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
        let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as *mut std::ffi::c_void)) }?;
        let display = smtc.DisplayUpdater()?;
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        smtc.SetIsEnabled(true)?;
        for f in [SystemMediaTransportControls::SetIsPlayEnabled, SystemMediaTransportControls::SetIsPauseEnabled, SystemMediaTransportControls::SetIsStopEnabled, SystemMediaTransportControls::SetIsNextEnabled, SystemMediaTransportControls::SetIsPreviousEnabled, SystemMediaTransportControls::SetIsFastForwardEnabled, SystemMediaTransportControls::SetIsRewindEnabled] {
            f(&smtc, true)?;
        }
        display.SetType(MediaPlaybackType::Music)?;
        let p = player.clone();
        smtc.ButtonPressed(&TypedEventHandler::new(move |_: &Option<SystemMediaTransportControls>, args: &Option<SystemMediaTransportControlsButtonPressedEventArgs>| {
            let Some(args) = args else { return Ok(()) };
            let b = args.Button()?;
            let seek = |by: f64| {
                let s = p.status().position;
                p.seek((s + by).max(0.0));
            };
            match b {
                SystemMediaTransportControlsButton::Play => p.resume(),
                SystemMediaTransportControlsButton::Pause | SystemMediaTransportControlsButton::Stop => p.pause(),
                SystemMediaTransportControlsButton::Next => p.next(true),
                SystemMediaTransportControlsButton::Previous => p.prev(),
                SystemMediaTransportControlsButton::FastForward => seek(10.0),
                SystemMediaTransportControlsButton::Rewind => seek(-10.0),
                _ => {}
            }
            Ok(())
        }))?;
        smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(move |_: &Option<SystemMediaTransportControls>, args: &Option<PlaybackPositionChangeRequestedEventArgs>| {
            if let Some(args) = args {
                player.seek(args.RequestedPlaybackPosition()?.Duration as f64 / 1e7);
            }
            Ok(())
        }))?;
        Ok(Self { smtc, display, timeline })
    }

    /// The song: title, artist, album, its cover file and length.
    pub fn set_song(&self, title: &str, artist: &str, album: &str, cover: Option<&str>, duration: f64) -> windows::core::Result<()> {
        let m = self.display.MusicProperties()?;
        m.SetTitle(&HSTRING::from(title))?;
        m.SetArtist(&HSTRING::from(artist))?;
        m.SetAlbumTitle(&HSTRING::from(album))?;
        // (a cover that can't be opened leaves the song shown without one)
        let thumb = cover.and_then(|c| StorageFile::GetFileFromPathAsync(&HSTRING::from(c)).ok()?.get().ok()).and_then(|f| RandomAccessStreamReference::CreateFromFile(&f).ok());
        match thumb {
            Some(t) => self.display.SetThumbnail(&t)?,
            None => self.display.SetThumbnail(None)?,
        }
        self.display.Update()?;
        self.timeline.SetStartTime(span(0.0))?;
        self.timeline.SetMinSeekTime(span(0.0))?;
        self.timeline.SetEndTime(span(duration))?;
        self.timeline.SetMaxSeekTime(span(duration))?;
        self.smtc.UpdateTimelineProperties(&self.timeline)
    }

    pub fn set_state(&self, playing: bool, position: f64) -> windows::core::Result<()> {
        self.smtc.SetPlaybackStatus(if playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused })?;
        self.timeline.SetPosition(span(position))?;
        self.smtc.UpdateTimelineProperties(&self.timeline)
    }
}
