//! The Win32 calls of Close Application, the only `unsafe` code of this
//! crate:
//!
//! - [`close_windows`]: `WM_CLOSE` to a process's visible, unowned
//!   top-level windows (what clicking their close button does), while a
//!   handle whose executable was checked pins the process id;
//! - [`terminate`]: `TerminateProcess`, on a handle whose executable is
//!   checked first, so a reused process id is never ended;
//! - [`aumid_of`]: a packaged application's `AppUserModelID`;
//! - [`session_of`]: the login session of a process;
//! - [`elevated`]: whether this process runs elevated.
//!
//! Every handle is closed by [`Handle`]'s `Drop`.
#![allow(unsafe_code)]

use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HWND, LPARAM};
use windows_sys::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    TerminateProcess,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
    WM_CLOSE,
};

use crate::process::ProcessInfo;
use crate::{AppError, codes};

/// An open process handle, closed when dropped.
struct Handle(HANDLE);

impl Handle {
    fn open(pid: u32, access: u32) -> Option<Self> {
        // SAFETY: OpenProcess takes plain values and returns a handle or
        // null; a null handle is never wrapped.
        let handle = unsafe { OpenProcess(access, 0, pid) };
        (!handle.is_null()).then_some(Self(handle))
    }

    /// The process's executable (`QueryFullProcessImageNameW`).
    fn image(&self) -> Option<PathBuf> {
        let mut buffer = vec![0u16; 32_768];
        let mut size = u32::try_from(buffer.len()).ok()?;
        // SAFETY: the buffer holds `size` UTF-16 units and outlives the
        // call; the function writes at most that many and updates `size`.
        let ok = unsafe {
            QueryFullProcessImageNameW(
                self.0,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &raw mut size,
            )
        };
        if ok == 0 {
            return None;
        }
        buffer.truncate(size as usize);
        Some(PathBuf::from(String::from_utf16_lossy(&buffer)))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// The login session of process `pid`.
pub fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    // SAFETY: `session` is a valid place for the result.
    let ok = unsafe { ProcessIdToSessionId(pid, &raw mut session) };
    (ok != 0).then_some(session)
}

/// The `AppUserModelID` of a packaged application's process; `None` for
/// any other process.
pub fn aumid_of(pid: u32) -> Option<String> {
    let handle = Handle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buffer = vec![0u16; 512];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: the buffer holds `length` UTF-16 units and outlives the call;
    // the function writes at most that many, including the terminator.
    let status =
        unsafe { GetApplicationUserModelId(handle.0, &raw mut length, buffer.as_mut_ptr()) };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

struct Search {
    pid: u32,
    windows: Vec<HWND>,
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> i32 {
    // SAFETY: `data` is the `Search` that `close_windows` passed and that
    // outlives the enumeration; the callback runs on the calling thread.
    let search = unsafe { &mut *(data as *mut Search) };
    let mut owner_pid = 0u32;
    // SAFETY: `window` is the window being enumerated; `owner_pid` is a
    // valid place for the result.
    unsafe { GetWindowThreadProcessId(window, &raw mut owner_pid) };
    if owner_pid == search.pid {
        // SAFETY: plain queries of the enumerated window.
        let visible = unsafe { IsWindowVisible(window) } != 0;
        let unowned = unsafe { GetWindow(window, GW_OWNER) }.is_null();
        if visible && unowned {
            search.windows.push(window);
        }
    }
    1
}

/// This process runs elevated (as an administrator with UAC's full
/// token); `false` when that cannot be read.
pub fn elevated() -> bool {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this process needs no closing; `token`
    // is a valid place for the opened token.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return false;
    }
    let token = Handle(token);
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut length = 0u32;
    let size = u32::try_from(std::mem::size_of::<TOKEN_ELEVATION>()).unwrap_or(4);
    // SAFETY: `elevation` is a TOKEN_ELEVATION of `size` bytes that
    // outlives the call.
    let ok = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&raw mut elevation).cast(),
            size,
            &raw mut length,
        )
    };
    ok != 0 && elevation.TokenIsElevated != 0
}

/// Posts `WM_CLOSE` to the visible, unowned top-level windows of
/// `process`. A handle to it is opened first and its executable checked,
/// and it stays open while the windows are found and asked, so the process
/// id cannot be given to another program meanwhile. `Ok(false)` when it
/// has no window (only force can end it) or is no longer that program.
pub fn close_windows(process: &ProcessInfo) -> Result<bool, AppError> {
    let pid = process.pid;
    let Some(pinned) = Handle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        return Ok(false);
    };
    let same = match (pinned.image(), &process.exe) {
        (Some(image), Some(exe)) => crate::process::same_file(&image, exe),
        _ => false,
    };
    if !same {
        return Ok(false);
    }
    let mut search = Search {
        pid,
        windows: Vec::new(),
    };
    // SAFETY: the callback only reads windows and writes to `search`,
    // which lives until EnumWindows returns.
    unsafe { EnumWindows(Some(collect), (&raw mut search) as LPARAM) };
    let mut posted = 0;
    for window in &search.windows {
        // SAFETY: posting a message to a window handle is always allowed;
        // a window that closed meanwhile makes the call fail harmlessly.
        if unsafe { PostMessageW(*window, WM_CLOSE, 0, 0) } != 0 {
            posted += 1;
        }
    }
    drop(pinned);
    if search.windows.is_empty() {
        Ok(false)
    } else if posted == 0 {
        Err(AppError::new(
            codes::CLOSE_FAILED,
            format!("the windows of process {pid} could not be asked to close"),
        ))
    } else {
        Ok(true)
    }
}

/// Ends `process` with `TerminateProcess`, after checking on the same
/// handle that it still runs the matched executable.
pub fn terminate(process: &ProcessInfo) -> Result<(), AppError> {
    let Some(handle) = Handle::open(
        process.pid,
        PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
    ) else {
        return Err(AppError::new(
            codes::CLOSE_FAILED,
            format!("process {} could not be opened to end it", process.pid),
        ));
    };
    let same = match (handle.image(), &process.exe) {
        (Some(image), Some(exe)) => crate::process::same_file(&image, exe),
        _ => false,
    };
    if !same {
        // Gone, and its id reused by another program: nothing to end.
        return Ok(());
    }
    // SAFETY: the handle is open with PROCESS_TERMINATE.
    if unsafe { TerminateProcess(handle.0, 1) } == 0 {
        return Err(AppError::new(
            codes::CLOSE_FAILED,
            format!("process {} could not be ended", process.pid),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_a_session_and_no_aumid() {
        let me = std::process::id();
        assert!(session_of(me).is_some());
        assert_eq!(aumid_of(me), None);
        let image = Handle::open(me, PROCESS_QUERY_LIMITED_INFORMATION)
            .and_then(|h| h.image())
            .unwrap();
        assert!(crate::process::same_file(
            &image,
            &std::env::current_exe().unwrap()
        ));
        // Whatever the runner's account, the answer is read without failing.
        let _ = elevated();
    }
}
