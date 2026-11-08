//! One process per user, and a way for a second launch to reach the first.
//!
//! A named mutex says whether we are first. The first instance also owns a
//! hidden message-only window with a known class; a second launch finds it
//! and posts one message, which the first answers by showing the popup. The
//! window lives on the main thread, whose messages Slint's event loop pumps.

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowW, MessageBoxW, PostMessageW,
    RegisterClassW, HWND_MESSAGE, MB_ICONWARNING, MB_OK, WINDOW_EX_STYLE, WM_APP, WNDCLASSW,
    WS_OVERLAPPED,
};

const MUTEX_NAME: PCWSTR = w!("Local\\com.reteren.rebuffer.native");
const SIGNAL_CLASS: PCWSTR = w!("RebufferNativeSignal");
const WM_SHOW_POPUP: u32 = WM_APP + 1;

/// Held for the life of the process; the OS releases it on exit either way.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // FFI: the handle came from CreateMutexW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// `Some` if this is the first instance.
pub fn acquire() -> Option<InstanceGuard> {
    // FFI: a named mutex in the session namespace; the name is a static
    // wide string.
    let handle = unsafe { CreateMutexW(None, true, MUTEX_NAME) }.ok()?;
    // FFI: GetLastError reports whether the mutex already existed.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // FFI: our handle to the existing mutex, not needed any further.
        unsafe {
            let _ = CloseHandle(handle);
        }
        return None;
    }
    Some(InstanceGuard(handle))
}

/// Asks the first instance to show its popup.
pub fn signal_first() {
    // FFI: FindWindowW with a static class name; a missing window is fine.
    if let Ok(hwnd) = unsafe { FindWindowW(SIGNAL_CLASS, PCWSTR::null()) } {
        // FFI: posting to a window of another process of ours.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_SHOW_POPUP, WPARAM(0), LPARAM(0));
        }
    }
}

/// True while the WebView build (`rebuffer.exe`) runs: it owns the same store
/// and the same hotkey.
pub fn webview_build_running() -> bool {
    // FFI: a process snapshot, walked and closed here.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case("rebuffer.exe") {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

/// Tells the user why the app is not starting. Before the tray icon exists
/// there is nowhere else to say it.
pub fn refuse(message: &str) {
    tracing::error!("{message}");
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    // FFI: a modal box with a NUL-terminated message and a static caption.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("Rebuffer"),
            MB_OK | MB_ICONWARNING,
        );
    }
}

thread_local! {
    static ON_SIGNAL: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The hidden window a second launch posts to. Dropping it stops listening.
pub struct SignalWindow(HWND);

impl Drop for SignalWindow {
    fn drop(&mut self) {
        // FFI: our own window, destroyed on the thread that created it.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// Starts listening for a second launch. Call on the main thread.
pub fn listen(on_signal: impl Fn() + 'static) -> Option<SignalWindow> {
    ON_SIGNAL.with(|s| *s.borrow_mut() = Some(Box::new(on_signal)));
    // FFI: class registration and a message-only window on this thread.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(signal_proc),
            hInstance: instance.into(),
            lpszClassName: SIGNAL_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            SIGNAL_CLASS,
            w!("Rebuffer"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
        .ok()?;
        Some(SignalWindow(hwnd))
    }
}

unsafe extern "system" fn signal_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_SHOW_POPUP {
        ON_SIGNAL.with(|s| {
            if let Some(f) = s.borrow().as_ref() {
                f();
            }
        });
        return LRESULT(0);
    }
    // FFI: everything else goes to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
