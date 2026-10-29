use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// 일정 한 건. 프런트엔드와 camelCase JSON으로 주고받는다.
///
/// 날짜는 "YYYY-MM-DD", 시간은 "HH:MM"(로컬 시간) 문자열로 저장한다.
/// 문자열 비교만으로 날짜 범위 판단이 되고, 시간대 변환 오류가 생기지 않는다.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start_date: String,
    pub end_date: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub memo: String,
    #[serde(default)]
    pub color: String,
    /// 알림: 시작 시각 기준 "몇 분 전" 목록. 0은 정시. 종일 일정은 당일 09:00이 시작 시각.
    #[serde(default)]
    pub reminders: Vec<i32>,
    /// 소속 캘린더. 로컬은 "local", 동기화 계정은 계정 ID.
    #[serde(default = "local_id")]
    pub calendar_id: String,
    /// 외부 캘린더에서의 원본 ID.
    #[serde(default)]
    pub remote_id: Option<String>,
    #[serde(default)]
    pub updated_at: i64,
    /// 삭제 표시. 서버에 삭제를 전달하기 전까지 남겨 둔다.
    #[serde(default)]
    pub deleted: bool,
    /// 읽기 전용(ICS 구독, 반복 일정에서 펼친 항목 등). 수정·삭제 불가.
    #[serde(default)]
    pub read_only: bool,
    /// 서버에 아직 반영하지 못한 로컬 변경이 있음.
    #[serde(default)]
    pub dirty: bool,
}

pub fn local_id() -> String {
    "local".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 위젯 배경 불투명도 0.2 ~ 1.0
    pub opacity: f64,
    /// "month": 현재 달 기준 / "rolling": 오늘 기준 위아래 2주
    pub month_mode: String,
    /// 0: 일요일 시작, 1: 월요일 시작
    pub week_start: u8,
    /// "day" | "week" | "month"
    pub view: String,
    /// 자동 동기화 주기(분). 0이면 끔.
    pub sync_minutes: u32,
    /// 동기화 계정이 없을 때 문서 폴더에 자동 백업
    pub backup_enabled: bool,
    pub notify_enabled: bool,
    /// "popup": 화면 오른쪽 위 팝업 창 / "windows": 윈도우 기본 알림
    pub notify_style: String,
    /// 테마 ID (THEME_IDS 중 하나)
    pub theme: String,
    /// 공휴일(쉬는 날) 표시
    pub show_holidays: bool,
    /// 기념일·24절기 표시
    pub show_special: bool,
    /// 음력(윤달 포함) 표시
    pub show_lunar: bool,
    /// 오늘 일정 패널(오늘/다가오는 일정/카드 줄) 표시
    pub show_panel: bool,
    /// 글자 크기 배율 0.8 ~ 1.6
    pub font_scale: f64,
    /// 글꼴 이름. 비어 있으면 기본 글꼴.
    pub font_family: String,
    /// 칸·일정 바탕 진하기 0 ~ 1 (0이면 바탕 없이 한 면으로 보임)
    pub surface: f64,
}

pub const THEME_IDS: [&str; 30] = [
    "midnight",
    "maple",
    "forest",
    "lavender",
    "graphite",
    "aurora",
    "hanji",
    "ocean",
    "sakura",
    "apple",
    "snow",
    "concrete",
    "wine",
    "peach",
    "sage",
    "sepia",
    "neon",
    "dancheong",
    "cobalt",
    "glacier",
    "ash",
    "slatemid",
    "fog",
    "caramel",
    "taupe",
    "pop",
    "blueprint",
    "candy",
    "lemonade",
    "lagoon",
];

impl Default for Settings {
    fn default() -> Self {
        Self {
            opacity: 0.85,
            month_mode: "month".to_string(),
            week_start: 0,
            view: "month".to_string(),
            sync_minutes: 15,
            backup_enabled: true,
            notify_enabled: true,
            notify_style: "popup".to_string(),
            theme: "midnight".to_string(),
            show_holidays: true,
            show_special: true,
            show_lunar: true,
            show_panel: true,
            font_scale: 1.0,
            font_family: String::new(),
            surface: 0.5,
        }
    }
}

/// 동기화 계정. 비밀번호·토큰이 들어 있으므로 프런트엔드에는 AccountInfo만 보낸다.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub id: String,
    /// "google" | "caldav" | "ics"
    pub kind: String,
    pub name: String,
    pub color: String,
    /// caldav: 서버 주소 / ics: 구독 주소 또는 파일 경로
    pub url: String,
    pub username: String,
    pub password: String,
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    /// caldav: 찾아낸 캘린더 주소들(첫 번째가 새 일정의 저장 위치)
    pub calendars: Vec<String>,
    pub last_sync: i64,
    pub last_error: String,
    pub enabled: bool,
}

impl Account {
    pub fn read_only(&self) -> bool {
        self.kind == "ics"
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub color: String,
    pub detail: String,
    pub last_sync: i64,
    pub last_error: String,
    pub read_only: bool,
}

impl From<&Account> for AccountInfo {
    fn from(a: &Account) -> Self {
        let detail = match a.kind.as_str() {
            "google" => "Google 캘린더".to_string(),
            "caldav" => format!("{} · {}", a.username, a.url),
            _ => a.url.clone(),
        };
        Self {
            id: a.id.clone(),
            kind: a.kind.clone(),
            name: a.name.clone(),
            color: a.color.clone(),
            detail,
            last_sync: a.last_sync,
            last_error: a.last_error.clone(),
            read_only: a.read_only(),
        }
    }
}

pub struct Store {
    dir: PathBuf,
    events: Mutex<Vec<Event>>,
    settings: Mutex<Settings>,
    accounts: Mutex<Vec<Account>>,
    fired: Mutex<Vec<String>>,
}

impl Store {
    pub fn load(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        let events: Vec<Event> = read_json(&dir.join("events.json")).unwrap_or_default();
        let settings: Settings = read_json(&dir.join("settings.json")).unwrap_or_default();
        let accounts: Vec<Account> = read_json(&dir.join("accounts.json")).unwrap_or_default();
        let fired: Vec<String> = read_json(&dir.join("fired.json")).unwrap_or_default();
        Self {
            dir,
            events: Mutex::new(events),
            settings: Mutex::new(settings),
            accounts: Mutex::new(accounts),
            fired: Mutex::new(fired),
        }
    }

    /// 데이터 폴더(%APPDATA%\com.widgetcalendar.app)
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn events_path(&self) -> PathBuf {
        self.dir.join("events.json")
    }

    // ───────── 일정 ─────────

    /// 삭제 표시가 없는 일정만 돌려준다.
    pub fn events(&self) -> Vec<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| !e.deleted)
            .cloned()
            .collect()
    }

    /// 삭제 표시까지 포함한 전체(동기화용).
    pub fn raw_events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    /// 사용자가 만든·고친 일정을 저장한다. 서버에서 온 필드(remoteId 등)는 클라이언트 값을 믿지 않는다.
    pub fn upsert(&self, mut ev: Event) -> Result<(), String> {
        ev.title = ev.title.trim().to_string();
        if ev.id.is_empty() || ev.title.is_empty() {
            return Err("id와 제목은 비워둘 수 없습니다.".into());
        }
        if ev.end_date < ev.start_date {
            ev.end_date = ev.start_date.clone();
        }
        if ev.calendar_id.trim().is_empty() {
            ev.calendar_id = local_id();
        }
        ev.deleted = false;
        ev.updated_at = now_ms();

        let mut list = self.events.lock().unwrap();
        match list.iter_mut().find(|e| e.id == ev.id) {
            Some(slot) => {
                if slot.read_only {
                    return Err("읽기 전용 일정은 수정할 수 없습니다.".into());
                }
                ev.remote_id = slot.remote_id.clone();
                ev.calendar_id = slot.calendar_id.clone();
                ev.read_only = false;
                ev.dirty = ev.calendar_id != "local";
                *slot = ev;
            }
            None => {
                ev.remote_id = None;
                ev.read_only = false;
                ev.dirty = ev.calendar_id != "local";
                list.push(ev);
            }
        }
        write_json(&self.events_path(), &*list)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut list = self.events.lock().unwrap();
        let Some(pos) = list.iter().position(|e| e.id == id) else {
            return Ok(());
        };
        if list[pos].read_only {
            return Err("읽기 전용 일정은 삭제할 수 없습니다.".into());
        }
        // 서버에 올라간 적이 있는 일정만 삭제 표시를 남기고, 나머지는 바로 지운다.
        if list[pos].calendar_id != "local" && list[pos].remote_id.is_some() {
            let e = &mut list[pos];
            e.deleted = true;
            e.dirty = true;
            e.updated_at = now_ms();
        } else {
            list.remove(pos);
        }
        write_json(&self.events_path(), &*list)
    }

    /// 동기화 엔진이 목록을 직접 고칠 때 쓴다. 잠금은 이 호출 동안만 잡는다.
    pub fn mutate_events<R>(&self, f: impl FnOnce(&mut Vec<Event>) -> R) -> Result<R, String> {
        let mut list = self.events.lock().unwrap();
        let r = f(&mut list);
        write_json(&self.events_path(), &*list)?;
        Ok(r)
    }

    /// 백업에서 복원: id가 같으면 수정 시각이 더 새로운 쪽을 남긴다.
    pub fn merge_events(&self, incoming: Vec<Event>) -> Result<usize, String> {
        self.mutate_events(|list| {
            let mut changed = 0;
            for mut ev in incoming {
                if ev.id.is_empty() || ev.title.trim().is_empty() {
                    continue;
                }
                ev.deleted = false;
                match list.iter_mut().find(|e| e.id == ev.id) {
                    Some(slot) if slot.updated_at >= ev.updated_at && !slot.deleted => {}
                    Some(slot) => {
                        *slot = ev;
                        changed += 1;
                    }
                    None => {
                        list.push(ev);
                        changed += 1;
                    }
                }
            }
            changed
        })
    }

    // ───────── 설정 ─────────

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_settings(&self, mut s: Settings) -> Result<Settings, String> {
        s.opacity = s.opacity.clamp(0.2, 1.0);
        if s.month_mode != "month" && s.month_mode != "rolling" {
            s.month_mode = "month".into();
        }
        if !["day", "week", "month"].contains(&s.view.as_str()) {
            s.view = "month".into();
        }
        s.week_start = s.week_start.min(1);
        s.sync_minutes = s.sync_minutes.min(1440);
        if s.notify_style != "popup" && s.notify_style != "windows" {
            s.notify_style = "popup".into();
        }
        if !THEME_IDS.contains(&s.theme.as_str()) {
            s.theme = "midnight".into();
        }
        s.font_scale = if s.font_scale.is_finite() { s.font_scale.clamp(0.8, 1.6) } else { 1.0 };
        s.surface = if s.surface.is_finite() { s.surface.clamp(0.0, 1.0) } else { 0.5 };
        // 글꼴 이름은 CSS에 들어가므로 글자·숫자·공백·-_., 만 남기고 60자로 자른다.
        s.font_family = s
            .font_family
            .chars()
            .filter(|c| c.is_alphanumeric() || " -_.,".contains(*c))
            .collect::<String>()
            .trim()
            .chars()
            .take(60)
            .collect();
        let mut cur = self.settings.lock().unwrap();
        *cur = s.clone();
        write_json(&self.dir.join("settings.json"), &*cur)?;
        Ok(s)
    }

    // ───────── 계정 ─────────

    pub fn accounts(&self) -> Vec<Account> {
        self.accounts.lock().unwrap().clone()
    }

    pub fn account(&self, id: &str) -> Option<Account> {
        self.accounts.lock().unwrap().iter().find(|a| a.id == id).cloned()
    }

    pub fn add_account(&self, acc: Account) -> Result<(), String> {
        let mut list = self.accounts.lock().unwrap();
        list.push(acc);
        write_json(&self.dir.join("accounts.json"), &*list)
    }

    pub fn update_account(&self, id: &str, f: impl FnOnce(&mut Account)) -> Result<(), String> {
        let mut list = self.accounts.lock().unwrap();
        if let Some(a) = list.iter_mut().find(|a| a.id == id) {
            f(a);
        }
        write_json(&self.dir.join("accounts.json"), &*list)
    }

    /// 계정을 지우고, 그 계정에서 가져온 일정도 이 PC에서 지운다(서버 데이터는 건드리지 않음).
    pub fn remove_account(&self, id: &str) -> Result<(), String> {
        {
            let mut list = self.accounts.lock().unwrap();
            list.retain(|a| a.id != id);
            write_json(&self.dir.join("accounts.json"), &*list)?;
        }
        self.mutate_events(|list| list.retain(|e| e.calendar_id != id))
    }

    // ───────── 알림 기록 ─────────

    /// 처음 기록하는 키면 true. 앱을 다시 켜도 같은 알림이 반복되지 않도록 파일에 남긴다.
    pub fn mark_fired(&self, key: &str) -> bool {
        let mut list = self.fired.lock().unwrap();
        if list.iter().any(|k| k == key) {
            return false;
        }
        list.push(key.to_string());
        if list.len() > 3000 {
            let cut = list.len() - 3000;
            list.drain(0..cut);
        }
        let _ = write_json(&self.dir.join("fired.json"), &*list);
        true
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 파일이 없으면 None. 파일이 깨졌으면 덮어써서 날리지 않도록 옆으로 치워두고 None.
fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(_) => {
            let _ = fs::rename(path, path.with_extension("json.corrupt"));
            None
        }
    }
}

/// 임시 파일에 쓴 뒤 교체한다. 쓰는 도중 꺼져도 기존 파일이 남는다.
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_text(path, &text)
}

pub fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("wc_store_{}", uuid::Uuid::new_v4().simple()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn ev(id: &str, title: &str) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start_date: "2026-10-30".into(),
            end_date: "2026-10-28".into(), // 일부러 뒤집어 둔다
            ..Default::default()
        }
    }

    #[test]
    fn upsert_cleans_input_and_survives_restart() {
        let dir = tmp();
        let s = Store::load(dir.clone());
        s.upsert(ev("1", "  회의  ")).unwrap();
        let e = &s.events()[0];
        assert_eq!(e.title, "회의");
        assert_eq!(e.end_date, "2026-10-30"); // 종료일이 앞서면 보정
        assert!(e.updated_at > 0);
        assert!(!e.dirty); // 로컬 일정은 서버에 올릴 것이 없다
        assert!(s.upsert(ev("2", "   ")).is_err());
        assert_eq!(Store::load(dir).events().len(), 1);
    }

    #[test]
    fn client_cannot_forge_server_fields() {
        let s = Store::load(tmp());
        let mut a = ev("1", "서버 일정");
        a.calendar_id = "acc".into();
        a.remote_id = Some("forged".into());
        a.read_only = true;
        s.upsert(a).unwrap();
        let e = &s.events()[0];
        assert_eq!(e.remote_id, None);
        assert!(!e.read_only);
        assert!(e.dirty); // 동기화 계정 소속이면 서버에 올려야 한다

        // 서버가 원본 ID를 부여한 뒤에는 클라이언트가 바꿀 수 없다.
        s.mutate_events(|l| l[0].remote_id = Some("real".into())).unwrap();
        let mut b = ev("1", "수정");
        b.calendar_id = "other".into();
        b.remote_id = Some("forged".into());
        s.upsert(b).unwrap();
        let e = &s.events()[0];
        assert_eq!((e.remote_id.as_deref(), e.calendar_id.as_str()), (Some("real"), "acc"));
    }

    #[test]
    fn readonly_events_are_protected() {
        let s = Store::load(tmp());
        let mut r = ev("r", "구독");
        r.read_only = true;
        s.mutate_events(|l| l.push(r)).unwrap();
        assert!(s.upsert(ev("r", "고치기")).is_err());
        assert!(s.remove("r").is_err());
        assert_eq!(s.events()[0].title, "구독");
    }

    #[test]
    fn remove_marks_synced_and_drops_local() {
        let s = Store::load(tmp());
        s.upsert(ev("local", "로컬")).unwrap();
        let mut a = ev("syn", "서버");
        a.calendar_id = "acc".into();
        s.upsert(a).unwrap();
        s.mutate_events(|l| l.iter_mut().find(|e| e.id == "syn").unwrap().remote_id = Some("r1".into())).unwrap();
        let mut b = ev("unsent", "아직 안 올림");
        b.calendar_id = "acc".into();
        s.upsert(b).unwrap();

        s.remove("local").unwrap();
        s.remove("syn").unwrap();
        s.remove("unsent").unwrap();
        assert_eq!(s.events().len(), 0);
        let raw = s.raw_events();
        assert_eq!(raw.len(), 1); // 서버에 올라간 적 있는 것만 삭제 표시로 남는다
        assert!(raw[0].deleted && raw[0].dirty && raw[0].remote_id.as_deref() == Some("r1"));
    }

    #[test]
    fn merge_keeps_newer_side() {
        let s = Store::load(tmp());
        s.upsert(ev("1", "현재")).unwrap();
        let now = s.events()[0].updated_at;
        let mut old = ev("1", "옛날 백업");
        old.updated_at = now - 1000;
        let mut newer = ev("2", "새 일정");
        newer.updated_at = 5;
        let mut newest = ev("1", "더 최근 백업");
        newest.updated_at = now + 1000;
        assert_eq!(s.merge_events(vec![old, newer.clone()]).unwrap(), 1); // id=2만 추가
        assert_eq!(s.events().iter().find(|e| e.id == "1").unwrap().title, "현재");
        assert_eq!(s.merge_events(vec![newest, newer]).unwrap(), 1); // id=1만 교체
        assert_eq!(s.events().iter().find(|e| e.id == "1").unwrap().title, "더 최근 백업");
    }

    #[test]
    fn fired_keys_are_recorded_once_and_persisted() {
        let dir = tmp();
        let s = Store::load(dir.clone());
        assert!(s.mark_fired("a|1"));
        assert!(!s.mark_fired("a|1"));
        assert!(!Store::load(dir).mark_fired("a|1"));
    }

    #[test]
    fn settings_are_clamped_and_old_files_load() {
        let dir = tmp();
        let s = Store::load(dir.clone());
        let saved = s
            .set_settings(Settings { opacity: 5.0, month_mode: "x".into(), week_start: 9, view: "?".into(), sync_minutes: 99999, ..Default::default() })
            .unwrap();
        assert_eq!((saved.opacity, saved.month_mode.as_str(), saved.week_start, saved.view.as_str(), saved.sync_minutes), (1.0, "month", 1, "month", 1440));
        let json = serde_json::to_string(&saved).unwrap();
        assert!(json.contains("monthMode") && json.contains("syncMinutes") && json.contains("notifyEnabled"));

        // 1단계에서 저장된 예전 파일(새 필드 없음)도 읽혀야 한다.
        fs::write(dir.join("settings.json"), r#"{"opacity":0.5,"monthMode":"rolling","weekStart":1}"#).unwrap();
        fs::write(dir.join("events.json"), r##"[{"id":"9","title":"t","startDate":"2026-01-01","endDate":"2026-01-01","color":"#fff","reminders":[10],"calendarId":"local","updatedAt":1,"deleted":false}]"##).unwrap();
        let s2 = Store::load(dir);
        assert_eq!((s2.settings().opacity, s2.settings().view.as_str(), s2.settings().sync_minutes), (0.5, "month", 15));
        assert_eq!(s2.events().len(), 1);
        assert!(!s2.events()[0].read_only);
    }

    #[test]
    fn corrupt_file_is_set_aside_not_overwritten() {
        let dir = tmp();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("events.json"), "{깨짐").unwrap();
        let s = Store::load(dir.clone());
        assert_eq!(s.events().len(), 0);
        assert!(dir.join("events.json.corrupt").exists());
    }

    #[test]
    fn remove_account_removes_its_events_only() {
        let s = Store::load(tmp());
        s.add_account(Account { id: "acc".into(), kind: "ics".into(), ..Default::default() }).unwrap();
        s.upsert(ev("l", "로컬")).unwrap();
        let mut r = ev("x", "구독");
        r.calendar_id = "acc".into();
        s.mutate_events(|l| l.push(r)).unwrap();
        s.remove_account("acc").unwrap();
        assert_eq!(s.accounts().len(), 0);
        assert_eq!(s.events().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), vec!["l"]);
    }
}
