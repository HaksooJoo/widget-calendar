//! 알림: 15초마다 일정을 훑어 알림 시각이 된 것을 윈도우 11 토스트로 띄운다.

use crate::store::{Event, Store};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime};
use std::time::Duration as StdDuration;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// 알림 시각이 지난 뒤에도 이 시간 안이면(절전 복귀 등) 알려 준다.
const LATE_GRACE_MIN: i64 = 30;

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(StdDuration::from_secs(15));
        check(&app);
    });
}

/// 일정의 시작 시각. 종일 일정은 당일 09:00.
fn start_of(ev: &Event) -> Option<NaiveDateTime> {
    let d = NaiveDate::parse_from_str(&ev.start_date, "%Y-%m-%d").ok()?;
    let t = if ev.all_day {
        NaiveTime::from_hms_opt(9, 0, 0)?
    } else {
        match ev.start_time.as_deref().and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok()) {
            Some(t) => t,
            None => NaiveTime::from_hms_opt(9, 0, 0)?,
        }
    };
    Some(d.and_time(t))
}

fn lead_text(min: i32) -> String {
    if min <= 0 {
        "지금 시작합니다".to_string()
    } else if min % 1440 == 0 {
        format!("{}일 후 시작", min / 1440)
    } else if min % 60 == 0 {
        format!("{}시간 후 시작", min / 60)
    } else {
        format!("{}분 후 시작", min)
    }
}

pub fn check(app: &AppHandle) {
    let store = app.state::<Store>();
    if !store.settings().notify_enabled {
        return;
    }
    let now = Local::now().naive_local();
    for ev in store.events() {
        if ev.reminders.is_empty() {
            continue;
        }
        let Some(start) = start_of(&ev) else { continue };
        for &m in &ev.reminders {
            let fire = start - Duration::minutes(m as i64);
            let grace = Duration::minutes(LATE_GRACE_MIN);
            if now >= fire && now <= fire + grace && now <= start + grace {
                let key = format!(
                    "{}|{}|{}|{}",
                    ev.id,
                    ev.start_date,
                    ev.start_time.clone().unwrap_or_default(),
                    m
                );
                if store.mark_fired(&key) {
                    let when = if ev.all_day || ev.start_time.is_none() {
                        "종일".to_string()
                    } else {
                        ev.start_time.clone().unwrap_or_default()
                    };
                    notify_event(app, &ev.title, &format!("{} · {}", lead_text(m), when), &ev.color, &ev.start_date);
                }
            }
        }
    }
}

/// 설정에 따라 오른쪽 위 팝업 또는 윈도우 기본 알림으로 알린다.
pub fn notify_event(app: &AppHandle, title: &str, body: &str, color: &str, date: &str) {
    if app.state::<Store>().settings().notify_style == "windows" {
        notify(app, title, body);
    } else {
        crate::toast::show(app, title, body, color, date);
    }
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("알림을 띄우지 못했습니다: {e}");
    }
}

/// 설치하지 않고 exe만 실행해도 토스트가 뜨도록, 앱 이름을 현재 사용자 레지스트리에 등록한다.
#[cfg(windows)]
pub fn register_app_id(id: &str, name: &str) {
    use std::os::windows::process::CommandExt;
    let key = format!("HKCU\\Software\\Classes\\AppUserModelId\\{id}");
    let _ = std::process::Command::new("reg")
        .args(["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", name, "/f"])
        .creation_flags(0x0800_0000)
        .status();
}

#[cfg(not(windows))]
pub fn register_app_id(_id: &str, _name: &str) {}
