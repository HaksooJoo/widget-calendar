//! 문서 폴더 백업: 문서\WidgetCalendar\ 안에 일정을 저장한다.
//!
//! - calendar-backup.json : 최신 전체 백업 (복원에 사용)
//! - calendar.ics         : 다른 캘린더 앱으로 옮길 수 있는 표준 파일
//! - backup-YYYY-MM-DD.json : 날짜별 사본(최근 14일)

use crate::store::{now_ms, write_json, write_text, Event, Store};
use crate::sync::ical;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const FOLDER: &str = "WidgetCalendar";
const KEEP_DAYS: usize = 14;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupFile {
    version: u32,
    exported_at: i64,
    events: Vec<Event>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub dir: String,
    pub last_backup: Option<i64>,
    pub enabled: bool,
    pub has_accounts: bool,
}

pub fn dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .document_dir()
        .map(|d| d.join(FOLDER))
        .map_err(|e| format!("문서 폴더를 찾을 수 없습니다: {e}"))
}

/// 백업을 만든다. force=false면 "동기화 계정이 없고 자동 백업을 켠 경우"에만 실행한다.
/// 만든 폴더 경로를 돌려주고, 건너뛰면 None.
pub fn run(app: &AppHandle, force: bool) -> Result<Option<PathBuf>, String> {
    let store = app.state::<Store>();
    if !force {
        let s = store.settings();
        if !s.backup_enabled || !store.accounts().is_empty() {
            return Ok(None);
        }
    }
    let dir = dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| format!("백업 폴더를 만들 수 없습니다: {e}"))?;

    let events = store.events();
    let file = BackupFile { version: 1, exported_at: now_ms(), events: events.clone() };
    write_json(&dir.join("calendar-backup.json"), &file)?;
    write_text(&dir.join("calendar.ics"), &ical::export_calendar(&events))?;

    // 날짜별 사본 + 오래된 사본 정리
    let day = chrono::Local::now().format("%Y-%m-%d");
    write_json(&dir.join(format!("backup-{day}.json")), &file)?;
    let mut snaps: Vec<String> = fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.starts_with("backup-") && n.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    snaps.sort();
    if snaps.len() > KEEP_DAYS {
        for old in &snaps[..snaps.len() - KEEP_DAYS] {
            let _ = fs::remove_file(dir.join(old));
        }
    }
    Ok(Some(dir))
}

pub fn info(app: &AppHandle) -> Result<BackupInfo, String> {
    let store = app.state::<Store>();
    let dir = dir(app)?;
    let last = fs::metadata(dir.join("calendar-backup.json"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    Ok(BackupInfo {
        dir: dir.to_string_lossy().to_string(),
        last_backup: last,
        enabled: store.settings().backup_enabled,
        has_accounts: !store.accounts().is_empty(),
    })
}

/// 백업 파일의 일정을 현재 일정과 합친다(같은 일정이면 더 최근에 고친 쪽을 남김).
pub fn restore(app: &AppHandle) -> Result<usize, String> {
    let path = dir(app)?.join("calendar-backup.json");
    let text = fs::read_to_string(&path).map_err(|_| "백업 파일이 없습니다. 먼저 백업을 만들어 주세요.".to_string())?;
    let file: BackupFile = serde_json::from_str(&text).map_err(|_| "백업 파일이 손상되었습니다.".to_string())?;
    app.state::<Store>().merge_events(file.events)
}

pub fn open_folder(app: &AppHandle) -> Result<(), String> {
    let dir = dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("xdg-open").arg(&dir).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}
