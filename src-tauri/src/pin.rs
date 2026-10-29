//! 위젯을 바탕화면에 붙여 둔다.
//!
//! - 항상 다른 창 뒤에 있도록 한다(창 옵션 alwaysOnBottom).
//! - Alt+Tab 목록에 나오지 않게 도구 창 속성(WS_EX_TOOLWINDOW)을 준다.
//! - Win+D(바탕화면 보기)로 최소화되면 곧바로 다시 보이게 한다.
//!   사용자가 트레이에서 일부러 숨긴 경우에는 건드리지 않는다.

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

static WANT_VISIBLE: AtomicBool = AtomicBool::new(true);

/// 위젯을 보이게 둘지 여부. 트레이에서 숨기면 false.
pub fn set_visible(v: bool) {
    WANT_VISIBLE.store(v, Ordering::SeqCst);
}

#[cfg(windows)]
mod ffi {
    use std::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        pub fn GetWindowLongPtrW(hwnd: *mut c_void, index: i32) -> isize;
        pub fn SetWindowLongPtrW(hwnd: *mut c_void, index: i32, value: isize) -> isize;
        pub fn SetWindowPos(
            hwnd: *mut c_void,
            insert_after: *mut c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        pub fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
        pub fn IsIconic(hwnd: *mut c_void) -> i32;
        pub fn IsWindowVisible(hwnd: *mut c_void) -> i32;
    }
}

#[cfg(windows)]
pub fn install(app: &AppHandle) {
    use std::{ffi::c_void, time::Duration};
    use tauri::Manager;

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x80;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;
    const HWND_BOTTOM: isize = 1;
    const SWP_NOSIZE_NOMOVE_NOACTIVATE: u32 = 0x1 | 0x2 | 0x10;

    let Some(win) = app.get_webview_window("main") else { return };
    std::thread::spawn(move || {
        let mut styled = false;
        loop {
            std::thread::sleep(Duration::from_millis(700));
            if !WANT_VISIBLE.load(Ordering::SeqCst) {
                continue;
            }
            let Ok(h) = win.hwnd() else { continue };
            let hwnd = (h.0 as isize) as *mut c_void;
            unsafe {
                if !styled {
                    let ex = ffi::GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    if ex & WS_EX_TOOLWINDOW == 0 {
                        ffi::SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW);
                        // 스타일 변경을 적용하려면 한 번 숨겼다가 다시 보여야 한다.
                        ffi::ShowWindow(hwnd, SW_HIDE);
                        ffi::ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        ffi::SetWindowPos(hwnd, HWND_BOTTOM as *mut c_void, 0, 0, 0, 0, SWP_NOSIZE_NOMOVE_NOACTIVATE);
                    }
                    styled = true;
                }
                if ffi::IsIconic(hwnd) != 0 || ffi::IsWindowVisible(hwnd) == 0 {
                    ffi::ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                    ffi::SetWindowPos(hwnd, HWND_BOTTOM as *mut c_void, 0, 0, 0, 0, SWP_NOSIZE_NOMOVE_NOACTIVATE);
                }
            }
        }
    });
}

#[cfg(not(windows))]
pub fn install(_app: &AppHandle) {}
