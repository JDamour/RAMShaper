//! Window-visibility queries for the UI's hidden-window rule (doc 06: a
//! hidden window must not drive high-frequency updates). The UI needs to
//! know its window is minimized from a plain thread — the Slint event loop
//! itself stops waking while occluded, so it cannot detect its own restore.
//! The unsafe FFI stays in this crate per the audited-boundary rule.

use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsIconic, HWND};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Find a top-level window by exact title. Returns the raw handle as an
/// opaque token for [`is_iconic`], or `None` when no such window exists.
/// With several windows sharing the title, one of them is returned.
pub fn find_window_by_title(title: &str) -> Option<isize> {
    let title = wide(title);
    // SAFETY: null class (any window), title pointer valid for the call;
    // the returned handle is used only as an opaque token.
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) };
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd.0 as isize)
    }
}

/// Whether the window is minimized (`IsIconic`). An invalid or closed
/// handle reads as `false`: a gone window simply stops being minimized.
pub fn is_iconic(hwnd: isize) -> bool {
    // SAFETY: plain read-only query on an opaque handle token.
    unsafe { IsIconic(HWND(hwnd as *mut _)) }.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_window_is_none_and_reads_not_minimized() {
        assert_eq!(find_window_by_title("ramshaper-window-that-does-not-exist"), None);
        assert!(!is_iconic(0));
        assert!(!is_iconic(-1));
    }
}
