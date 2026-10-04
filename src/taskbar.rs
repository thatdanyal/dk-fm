//! Windows: previous / play-pause / next buttons in the window's taskbar thumbnail (hover DK.FM
//! on the taskbar), like Spotify has. Uses the shell's ITaskbarList3 (called through its vtable:
//! windows-sys has the functions and structs, not COM interfaces) and a window subclass to hear
//! the clicks. The buttons are added again whenever Explorer recreates the taskbar button (after
//! the window comes back from the tray, or Explorer restarts).
use crate::player::Player;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass, TaskbarList, THBF_ENABLED, THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIcon, RegisterWindowMessageW, HICON, WM_COMMAND};

const IID_ITASKBARLIST3: GUID = GUID::from_u128(0xea1afb91_9e28_4b86_90e9_9e9f8a5eefaf);
const PREV: u32 = 1;
const PLAY: u32 = 2;
const NEXT: u32 = 3;

/// ITaskbarList3's methods, in order (IUnknown, ITaskbarList, ITaskbarList2, ITaskbarList3).
#[repr(C)]
struct Vtbl {
    query_interface: usize,
    add_ref: usize,
    release: usize,
    hr_init: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    add_tab: usize,
    delete_tab: usize,
    activate_tab: usize,
    set_active_alt: usize,
    mark_fullscreen_window: usize,
    set_progress_value: usize,
    set_progress_state: usize,
    register_tab: usize,
    unregister_tab: usize,
    set_tab_order: usize,
    set_tab_active: usize,
    thumb_bar_add_buttons: unsafe extern "system" fn(*mut c_void, HWND, u32, *const THUMBBUTTON) -> HRESULT,
    thumb_bar_update_buttons: unsafe extern "system" fn(*mut c_void, HWND, u32, *const THUMBBUTTON) -> HRESULT,
}

static LIST: AtomicIsize = AtomicIsize::new(0);
static HWND_: AtomicIsize = AtomicIsize::new(0);
static CREATED_MSG: AtomicU32 = AtomicU32::new(0);
static PLAYING: AtomicBool = AtomicBool::new(false);
static ADDED: AtomicBool = AtomicBool::new(false);
static PLAYER: OnceLock<Arc<Player>> = OnceLock::new();
/// prev, play, pause, next (HICONs)
static ICONS: OnceLock<[isize; 4]> = OnceLock::new();

/// Set up the buttons for DK.FM's window (call once, on the window's thread).
pub fn init(hwnd: isize, player: Arc<Player>) {
    if hwnd == 0 || PLAYER.set(player).is_err() {
        return;
    }
    HWND_.store(hwnd, Ordering::Relaxed);
    unsafe {
        let _ = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let name: Vec<u16> = "TaskbarButtonCreated\0".encode_utf16().collect();
        CREATED_MSG.store(RegisterWindowMessageW(name.as_ptr()), Ordering::Relaxed);
        SetWindowSubclass(hwnd as HWND, Some(subclass), 0xD4F, 0);
    }
    let _ = ICONS.set([icon(Glyph::Prev), icon(Glyph::Play), icon(Glyph::Pause), icon(Glyph::Next)]);
    add_buttons(); // (in case the taskbar button already exists)
}

/// Show ⏸ while playing, ▶ while paused.
pub fn set_playing(playing: bool) {
    if PLAYING.swap(playing, Ordering::Relaxed) == playing || !ADDED.load(Ordering::Relaxed) {
        return;
    }
    let b = [button(PLAY)];
    unsafe {
        if let Some(l) = list() {
            ((*vtbl(l)).thumb_bar_update_buttons)(l, HWND_.load(Ordering::Relaxed) as HWND, 1, b.as_ptr());
        }
    }
}

unsafe fn vtbl(l: *mut c_void) -> *const Vtbl {
    *(l as *const *const Vtbl)
}

/// The shell's taskbar object (created on first use).
fn list() -> Option<*mut c_void> {
    let l = LIST.load(Ordering::Relaxed);
    if l != 0 {
        return Some(l as *mut c_void);
    }
    unsafe {
        let mut p: *mut c_void = std::ptr::null_mut();
        if CoCreateInstance(&TaskbarList, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ITASKBARLIST3, &mut p) < 0 || p.is_null() {
            return None;
        }
        if ((*vtbl(p)).hr_init)(p) < 0 {
            return None;
        }
        LIST.store(p as isize, Ordering::Relaxed);
        Some(p)
    }
}

fn button(id: u32) -> THUMBBUTTON {
    let icons = ICONS.get().copied().unwrap_or([0; 4]);
    let playing = PLAYING.load(Ordering::Relaxed);
    let (ico, tip) = match id {
        PREV => (icons[0], "Previous"),
        NEXT => (icons[3], "Next"),
        _ if playing => (icons[2], "Pause"),
        _ => (icons[1], "Play"),
    };
    let mut b: THUMBBUTTON = unsafe { std::mem::zeroed() };
    b.dwMask = THB_ICON | THB_TOOLTIP | THB_FLAGS;
    b.iId = id;
    b.hIcon = ico as HICON;
    b.dwFlags = THBF_ENABLED;
    for (i, c) in tip.encode_utf16().take(259).enumerate() {
        b.szTip[i] = c;
    }
    b
}

fn add_buttons() {
    let hwnd = HWND_.load(Ordering::Relaxed);
    let Some(l) = list() else { return };
    let b = [button(PREV), button(PLAY), button(NEXT)];
    unsafe {
        // after Explorer recreated the button the old ones are gone: adding works again (adding
        // twice to the same button fails, so try an update too)
        let ok = ((*vtbl(l)).thumb_bar_add_buttons)(l, hwnd as HWND, 3, b.as_ptr()) >= 0 || ((*vtbl(l)).thumb_bar_update_buttons)(l, hwnd as HWND, 3, b.as_ptr()) >= 0;
        ADDED.store(ok, Ordering::Relaxed);
    }
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    let created = CREATED_MSG.load(Ordering::Relaxed);
    if created != 0 && msg == created {
        add_buttons();
    } else if msg == WM_COMMAND && ((wparam >> 16) & 0xffff) as u32 == THBN_CLICKED {
        if let Some(p) = PLAYER.get().cloned() {
            let id = (wparam & 0xffff) as u32;
            // (not inside the window procedure: the player takes its own locks)
            std::thread::spawn(move || match id {
                PREV => p.prev(),
                NEXT => p.next(true),
                _ => p.toggle(),
            });
            return 0;
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

#[derive(Clone, Copy)]
enum Glyph {
    Prev,
    Play,
    Pause,
    Next,
}

/// A white 20 px glyph as an icon (drawn 4x oversampled for smooth edges).
fn icon(g: Glyph) -> isize {
    const N: usize = 20;
    let inside = |x: f32, y: f32| -> bool {
        let tri_right = |x0: f32, x1: f32| x >= x0 && x <= x1 && (y - 10.0).abs() <= (x1 - x) * 0.62;
        let tri_left = |x0: f32, x1: f32| x >= x0 && x <= x1 && (y - 10.0).abs() <= (x - x0) * 0.62;
        let bar = |x0: f32, x1: f32| x >= x0 && x <= x1 && (4.0..=16.0).contains(&y);
        match g {
            Glyph::Play => tri_right(5.0, 16.0),
            Glyph::Pause => bar(5.0, 8.5) || bar(11.5, 15.0),
            Glyph::Prev => bar(4.0, 6.0) || tri_left(6.5, 16.0),
            Glyph::Next => tri_right(4.0, 13.5) || bar(14.0, 16.0),
        }
    };
    let mut bgra = vec![0u8; N * N * 4];
    for py in 0..N {
        for px in 0..N {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0) {
                        hit += 1;
                    }
                }
            }
            let a = (hit * 255 / 16) as u8;
            let i = (py * N + px) * 4;
            bgra[i..i + 4].copy_from_slice(&[255, 255, 255, a]);
        }
    }
    let and = vec![0u8; N * 4]; // 20 px rows padded to 4 bytes; all zero = use the alpha
    unsafe { CreateIcon(std::ptr::null_mut(), N as i32, N as i32, 1, 32, and.as_ptr(), bgra.as_ptr()) as isize }
}
