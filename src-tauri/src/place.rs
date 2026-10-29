//! 창 위치: 새 창은 위젯이 놓인 모니터에 띄운다. (듀얼 모니터에서 다른 모니터에 뜨는 문제 방지)
//! 모니터마다 배율(DPI)이 다를 수 있으므로 물리 좌표로 계산한다.

use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

/// 위젯(메인 창)이 놓인 모니터. 알 수 없으면 기본 모니터.
pub fn widget_monitor(app: &AppHandle) -> Option<Monitor> {
    app.get_webview_window("main")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// 논리 크기(w, h)의 창을 위젯이 있는 모니터의 작업 영역 가운데에 놓는다.
/// 작업 영역보다 크면 그 안에 들어가도록 줄인다.
pub fn center_on_widget_monitor(app: &AppHandle, win: &WebviewWindow, w: f64, h: f64) {
    let Some(m) = widget_monitor(app) else { return };
    let s = m.scale_factor();
    let wa = m.work_area();
    let (aw, ah) = (wa.size.width as f64, wa.size.height as f64);
    let pw = (w * s).round().min(aw - 24.0 * s).max(200.0);
    let ph = (h * s).round().min(ah - 24.0 * s).max(200.0);
    let x = wa.position.x as f64 + (aw - pw) / 2.0;
    let y = wa.position.y as f64 + (ah - ph) / 2.0;
    let _ = win.set_size(PhysicalSize::new(pw as u32, ph as u32));
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}
