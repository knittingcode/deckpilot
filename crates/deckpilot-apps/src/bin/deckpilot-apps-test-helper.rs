//! A tiny program for the tests of `deckpilot-apps` (`tests/real.rs`): the
//! only program those tests ever open or close. It is never shipped.
//!
//! - `sleep`: waits (at most two minutes) until it is ended;
//! - `stubborn`: the same, but ignores `SIGTERM` (Unix), like a program
//!   that does not quit when asked;
//! - `window`: (Windows) shows a window and exits when it is closed.
//!
//! It prints `ready` once it is set up.
#![allow(unsafe_code)]

use std::io::Write as _;
use std::time::Duration;

fn ready() {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "ready");
    let _ = out.flush();
}

fn sleep() {
    ready();
    std::thread::sleep(Duration::from_secs(120));
}

#[cfg(unix)]
fn ignore_sigterm() {
    // SAFETY: SIG_IGN is a valid disposition for SIGTERM, set before any
    // other thread exists.
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
}

#[cfg(windows)]
fn window() {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG,
        PostQuitMessage, RegisterClassW, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_DESTROY {
            // SAFETY: called on the thread that owns the message loop.
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        // SAFETY: the default handling of a message of our own window;
        // WM_CLOSE destroys it.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    let class: Vec<u16> = "DeckPilotAppsTestHelper\0".encode_utf16().collect();
    // SAFETY: plain Win32 calls with valid, NUL-terminated strings that
    // outlive them; the message loop runs on this thread.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let definition = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..WNDCLASSW::default()
        };
        RegisterClassW(&raw const definition);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            200,
            100,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if window.is_null() {
            std::process::exit(3);
        }
        ready();
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&raw const message);
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("sleep") => sleep(),
        Some("stubborn") => {
            #[cfg(unix)]
            ignore_sigterm();
            sleep();
        }
        #[cfg(windows)]
        Some("window") => window(),
        _ => std::process::exit(2),
    }
}
