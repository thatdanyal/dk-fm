//! Voice search (Windows): the microphone in the search bar opens Windows voice typing (the Win+H
//! bar) on the search box, so what you say is typed there and searched. Windows does the hearing
//! with its own recogniser (the one that works in every other app), DK.FM carries no speech model
//! and nothing runs until you press it.
//!
//! (DK.FM used to ask Windows' speech API itself, but that answers "unknown" to apps that aren't
//! installed from the Microsoft Store on some Windows versions.)

/// Whether this computer can do it (Windows only for now).
pub fn available() -> bool {
    cfg!(windows)
}

/// Open Windows voice typing on the focused text box (presses Win+H for you).
#[cfg(windows)]
pub fn start_typing() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_H, VK_LWIN};
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { 0 }, time: 0, dwExtraInfo: 0 } },
    };
    let keys = [key(VK_LWIN, false), key(VK_H, false), key(VK_H, true), key(VK_LWIN, true)];
    let sent = unsafe { SendInput(keys.len() as u32, keys.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    sent as usize == keys.len()
}

#[cfg(not(windows))]
pub fn start_typing() -> bool {
    false
}
