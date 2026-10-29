//! 알림 팝업: 화면 오른쪽 위에 작은 창을 직접 띄운다.
//! 여러 개가 겹치면 위에서 아래로 쌓고, 하나가 닫히면 나머지를 다시 정렬한다.

use crate::sync::urlenc;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

const W: f64 = 380.0;
const H: f64 = 112.0;
const GAP: f64 = 4.0;
const MARGIN: f64 = 12.0;
const MAX_ACTIVE: usize = 5;

static ACTIVE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SEQ: AtomicU64 = AtomicU64::new(1);

/// n번째 알림이 놓일 위치와 크기(물리 좌표). 위젯이 있는 모니터의 작업 영역 오른쪽 위 기준.
fn slot(app: &AppHandle, idx: usize) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let (s, ax, ay, aw) = match crate::place::widget_monitor(app) {
        Some(m) => {
            let wa = m.work_area();
            (m.scale_factor(), wa.position.x as f64, wa.position.y as f64, wa.size.width as f64)
        }
        None => (1.0, 0.0, 0.0, 1280.0),
    };
    let (w, h) = ((W * s).round(), (H * s).round());
    let x = ax + aw - w - MARGIN * s;
    let y = ay + (MARGIN + idx as f64 * (H + GAP)) * s;
    (PhysicalPosition::new(x.round() as i32, y.round() as i32), PhysicalSize::new(w as u32, h as u32))
}

pub fn show(app: &AppHandle, title: &str, body: &str, color: &str, date: &str) {
    let app = app.clone();
    let (title, body, color, date) = (title.to_string(), body.to_string(), color.to_string(), date.to_string());
    tauri::async_runtime::spawn(async move {
        if let Err(e) = build(&app, &title, &body, &color, &date) {
            eprintln!("알림 팝업을 띄우지 못했습니다: {e}");
        }
    });
}

fn build(app: &AppHandle, title: &str, body: &str, color: &str, date: &str) -> Result<(), String> {
    // 너무 많이 쌓이면 가장 오래된 것부터 닫는다.
    let oldest = {
        let a = ACTIVE.lock().unwrap();
        if a.len() >= MAX_ACTIVE { a.first().cloned() } else { None }
    };
    if let Some(label) = oldest {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.close();
        }
    }

    let label = format!("toast-{}", SEQ.fetch_add(1, Ordering::SeqCst));
    let idx = {
        let mut a = ACTIVE.lock().unwrap();
        a.push(label.clone());
        a.len() - 1
    };
    let (pos, size) = slot(app, idx);
    let url = format!(
        "toast.html?title={}&body={}&color={}&date={}",
        urlenc(title),
        urlenc(body),
        urlenc(color),
        urlenc(date)
    );
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title("일정 알림")
        .inner_size(W, H)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .build();
    match built {
        // 모니터마다 배율이 다를 수 있어, 만든 직후 물리 좌표로 위젯이 있는 모니터에 옮긴다.
        Ok(w) => {
            let _ = w.set_size(size);
            let _ = w.set_position(pos);
        }
        Err(e) => {
            ACTIVE.lock().unwrap().retain(|l| *l != label);
            return Err(e.to_string());
        }
    }
    Ok(())
}

/// 닫힌 알림을 목록에서 빼고 나머지를 위로 당겨 정렬한다.
pub fn on_destroyed(app: &AppHandle, label: &str) {
    let rest = {
        let mut a = ACTIVE.lock().unwrap();
        a.retain(|l| l != label);
        a.clone()
    };
    for (i, l) in rest.iter().enumerate() {
        if let Some(w) = app.get_webview_window(l) {
            let (pos, _) = slot(app, i);
            let _ = w.set_position(pos);
        }
    }
}
