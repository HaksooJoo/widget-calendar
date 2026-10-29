//! 외부 캘린더 동기화 엔진.
//!
//! 한 번의 동기화 = ① 이 PC에서 바뀐 일정(dirty)을 서버에 올리고 ② 서버의 현재 상태를 받아 합친다.
//! 서버 조회 구간은 오늘 기준 과거 90일 ~ 미래 365일이다.

pub mod caldav;
pub mod google;
pub mod ical;

use crate::store::{now_ms, Event, Store};
use chrono::{Duration, Local, NaiveDate};
use std::{collections::HashSet, sync::Mutex, time::Duration as StdDuration};
use tauri::{AppHandle, Emitter, Manager};

static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 같은 계정을 동시에 두 번 동기화하지 않도록 하는 잠금
struct Guard(String);

impl Guard {
    fn acquire(id: &str) -> Option<Guard> {
        let mut r = RUNNING.lock().unwrap();
        if r.iter().any(|x| x == id) {
            None
        } else {
            r.push(id.to_string());
            Some(Guard(id.to_string()))
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        RUNNING.lock().unwrap().retain(|x| *x != self.0);
    }
}

pub fn window() -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    (today - Duration::days(90), today + Duration::days(365))
}

pub fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn http_client(manual_redirects: bool) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("WidgetCalendar/1.0")
        .timeout(StdDuration::from_secs(40))
        .redirect(if manual_redirects {
            reqwest::redirect::Policy::none()
        } else {
            reqwest::redirect::Policy::limited(5)
        })
        .build()
        .map_err(|e| e.to_string())
}

pub fn urlenc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 기본 브라우저로 주소를 연다.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x0800_0000) // 콘솔 창 숨김
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

/// 서버에서 받은 현재 상태를 로컬 일정과 합친다.
///
/// - 서버에 있는 일정: 새로 추가하거나 덮어쓴다(로컬에 올리지 못한 변경이 있으면 그쪽을 우선).
/// - 조회 구간 안에서 서버에서 사라진 일정: 이 PC에서도 지운다.
pub fn reconcile(
    store: &Store,
    account_id: &str,
    remote: Vec<Event>,
    ws: NaiveDate,
    we: NaiveDate,
) -> Result<(), String> {
    let (ws, we) = (fmt_date(ws), fmt_date(we));
    store.mutate_events(|list| {
        let remote_ids: HashSet<String> = remote.iter().filter_map(|e| e.remote_id.clone()).collect();

        for mut r in remote {
            let Some(rid) = r.remote_id.clone() else { continue };
            r.calendar_id = account_id.to_string();
            r.dirty = false;
            let pos = list
                .iter()
                .position(|e| e.calendar_id == account_id && e.remote_id.as_deref() == Some(rid.as_str()));
            match pos {
                Some(i) => {
                    if list[i].dirty || list[i].deleted {
                        continue; // 이 PC의 변경이 먼저 서버에 올라가야 한다
                    }
                    r.id = list[i].id.clone();
                    list[i] = r;
                }
                None => {
                    r.id = format!("{}:{}", account_id, rid);
                    list.push(r);
                }
            }
        }

        list.retain(|e| {
            let gone = e.calendar_id == account_id
                && !e.dirty
                && !e.deleted
                && e.remote_id.as_ref().map_or(false, |r| !remote_ids.contains(r))
                && e.start_date <= we
                && e.end_date >= ws;
            !gone
        });
    })
}

/// 서버에 올린 뒤 로컬 표시를 정리한다. 올리는 사이 사용자가 또 고쳤다면(updated_at이 다르면) 그대로 둔다.
pub fn mark_pushed(store: &Store, ev: &Event, remote_id: Option<String>) -> Result<(), String> {
    store.mutate_events(|list| {
        if ev.deleted {
            list.retain(|e| !(e.id == ev.id && e.updated_at == ev.updated_at));
        } else if let Some(e) = list.iter_mut().find(|e| e.id == ev.id) {
            if let Some(r) = remote_id {
                e.remote_id = Some(r);
            }
            if e.updated_at == ev.updated_at {
                e.dirty = false;
            }
        }
    })
}

/// 이 계정에서 서버에 올려야 할 변경 목록
pub fn pending(store: &Store, account_id: &str) -> Vec<Event> {
    store
        .raw_events()
        .into_iter()
        .filter(|e| e.calendar_id == account_id && e.dirty && !e.read_only)
        .collect()
}

pub async fn fetch_text(url: &str) -> Result<String, String> {
    let u = url.trim();
    let lower = u.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("webcal://") {
        let target = if lower.starts_with("webcal://") {
            format!("https://{}", &u[9..])
        } else {
            u.to_string()
        };
        let resp = http_client(false)?
            .get(&target)
            .send()
            .await
            .map_err(|e| format!("주소에 연결할 수 없습니다: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("서버가 오류를 돌려주었습니다 ({}).", resp.status().as_u16()));
        }
        resp.text().await.map_err(|e| e.to_string())
    } else {
        std::fs::read_to_string(u.trim_matches('"')).map_err(|e| format!("파일을 읽을 수 없습니다: {e}"))
    }
}

async fn sync_ics(store: &Store, acc: &crate::store::Account) -> Result<(), String> {
    let text = fetch_text(&acc.url).await?;
    if !text.to_uppercase().contains("BEGIN:VCALENDAR") {
        return Err("올바른 캘린더(ICS) 형식이 아닙니다.".into());
    }
    let (ws, we) = window();
    let parsed = ical::parse(&text, ws, we);
    let remote: Vec<Event> = parsed
        .items
        .into_iter()
        .map(|p| {
            let mut ev = p.event;
            ev.remote_id = Some(format!("{}@{}", p.uid, ev.start_date));
            ev.color = acc.color.clone();
            ev.read_only = true;
            ev
        })
        .collect();
    reconcile(store, &acc.id, remote, ws, we)
}

/// 계정 하나를 동기화하고 결과(마지막 동기화 시각, 오류)를 계정에 기록한다.
pub async fn sync_store(store: &Store, id: &str) -> Result<(), String> {
    let acc = store.account(id).ok_or_else(|| "계정을 찾을 수 없습니다.".to_string())?;
    if !acc.enabled {
        return Ok(());
    }
    let Some(_guard) = Guard::acquire(id) else {
        return Err("이미 동기화 중입니다.".into());
    };
    let res = match acc.kind.as_str() {
        "google" => google::sync(store, &acc).await,
        "caldav" => caldav::sync(store, &acc).await,
        "ics" => sync_ics(store, &acc).await,
        _ => Err("알 수 없는 계정 종류입니다.".to_string()),
    };
    let err = res.as_ref().err().cloned().unwrap_or_default();
    let _ = store.update_account(id, |a| {
        a.last_sync = now_ms();
        a.last_error = err;
    });
    res
}

pub async fn sync_and_notify(app: &AppHandle, id: &str) -> Result<(), String> {
    let store = app.state::<Store>();
    let r = sync_store(&store, id).await;
    let _ = app.emit("events-changed", ());
    let _ = app.emit("accounts-changed", ());
    if r.is_ok() {
        let _ = crate::backup::run(app, false); // 동기화 계정이 있으면 건너뜀(설정에 따라)
    }
    r
}

/// 사용자가 일정을 고친 뒤 잠깐 있다가 해당 계정을 동기화한다.
pub fn schedule(app: &AppHandle, account_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(StdDuration::from_secs(3)).await;
        let _ = sync_and_notify(&app, &account_id).await;
    });
}

/// 시작 5초 뒤 한 번, 이후 설정한 주기마다 자동 동기화
pub fn spawn_auto(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(StdDuration::from_secs(5)).await;
        let ids: Vec<String> = app.state::<Store>().accounts().into_iter().map(|a| a.id).collect();
        for id in ids {
            let _ = sync_and_notify(&app, &id).await;
        }
        loop {
            tokio::time::sleep(StdDuration::from_secs(30)).await;
            let (mins, accounts) = {
                let st = app.state::<Store>();
                (st.settings().sync_minutes, st.accounts())
            };
            if mins == 0 {
                continue;
            }
            for acc in accounts {
                if acc.enabled && now_ms() - acc.last_sync >= mins as i64 * 60_000 {
                    let _ = sync_and_notify(&app, &acc.id).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::load(std::env::temp_dir().join(format!("wc_sync_{}", uuid::Uuid::new_v4().simple())))
    }

    fn remote(rid: &str, title: &str, start: &str) -> Event {
        Event {
            title: title.into(),
            start_date: start.into(),
            end_date: start.into(),
            remote_id: Some(rid.into()),
            ..Default::default()
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn reconcile_adds_updates_deletes() {
        let s = store();
        let (ws, we) = (d("2026-10-01"), d("2026-12-31"));
        reconcile(&s, "acc", vec![remote("a", "A", "2026-10-05"), remote("b", "B", "2026-10-06")], ws, we).unwrap();
        assert_eq!(s.events().len(), 2);
        let id_a = s.events().iter().find(|e| e.title == "A").unwrap().id.clone();

        // A는 제목이 바뀌고(로컬 id 유지), B는 서버에서 사라졌고, C는 새로 생겼다.
        reconcile(&s, "acc", vec![remote("a", "A2", "2026-10-05"), remote("c", "C", "2026-10-07")], ws, we).unwrap();
        let titles: Vec<String> = { let mut t: Vec<String> = s.events().iter().map(|e| e.title.clone()).collect(); t.sort(); t };
        assert_eq!(titles, vec!["A2", "C"]);
        assert_eq!(s.events().iter().find(|e| e.title == "A2").unwrap().id, id_a);
    }

    #[test]
    fn reconcile_respects_window_other_accounts_and_pending_changes() {
        let s = store();
        let (ws, we) = (d("2026-10-01"), d("2026-12-31"));
        // 조회 구간 밖의 옛 일정, 다른 계정 일정, 로컬 일정은 서버 목록에 없어도 지우지 않는다.
        let mut old = remote("old", "옛날", "2025-01-01");
        old.calendar_id = "acc".into();
        let mut other = remote("o", "다른 계정", "2026-10-05");
        other.calendar_id = "other".into();
        let local = Event { id: "l".into(), title: "로컬".into(), start_date: "2026-10-05".into(), end_date: "2026-10-05".into(), calendar_id: "local".into(), ..Default::default() };
        // 서버에 올리지 못한 로컬 수정(dirty)은 서버 값으로 덮어쓰지 않는다.
        let mut dirty = remote("dd", "내가 고친 제목", "2026-10-08");
        dirty.id = "dirty1".into();
        dirty.calendar_id = "acc".into();
        dirty.dirty = true;
        s.mutate_events(|l| l.extend([old, other, local, dirty])).unwrap();

        reconcile(&s, "acc", vec![remote("dd", "서버의 제목", "2026-10-08")], ws, we).unwrap();
        let mut got: Vec<String> = s.events().iter().map(|e| e.title.clone()).collect();
        got.sort();
        assert_eq!(got, vec!["내가 고친 제목", "다른 계정", "로컬", "옛날"]);
    }

    #[test]
    fn reconcile_does_not_resurrect_pending_deletes() {
        let s = store();
        let (ws, we) = (d("2026-10-01"), d("2026-12-31"));
        let mut gone = remote("x", "지운 일정", "2026-10-05");
        gone.id = "g".into();
        gone.calendar_id = "acc".into();
        gone.deleted = true;
        gone.dirty = true;
        s.mutate_events(|l| l.push(gone)).unwrap();
        reconcile(&s, "acc", vec![remote("x", "지운 일정", "2026-10-05")], ws, we).unwrap();
        assert_eq!(s.events().len(), 0);
        assert_eq!(s.raw_events().len(), 1);
    }

    #[test]
    fn mark_pushed_handles_create_delete_and_concurrent_edit() {
        let s = store();
        let mk = |id: &str, ts: i64| Event { id: id.into(), title: id.into(), start_date: "2026-10-05".into(), end_date: "2026-10-05".into(), calendar_id: "acc".into(), dirty: true, updated_at: ts, ..Default::default() };
        s.mutate_events(|l| l.extend([mk("new", 10), mk("edited", 10)])).unwrap();
        let snapshot_new = s.raw_events().into_iter().find(|e| e.id == "new").unwrap();
        let snapshot_edit = s.raw_events().into_iter().find(|e| e.id == "edited").unwrap();

        // 올리는 사이 사용자가 "edited"를 또 고쳤다 → dirty를 유지해야 한다.
        s.mutate_events(|l| l.iter_mut().find(|e| e.id == "edited").unwrap().updated_at = 99).unwrap();
        mark_pushed(&s, &snapshot_new, Some("rid1".into())).unwrap();
        mark_pushed(&s, &snapshot_edit, None).unwrap();
        let raw = s.raw_events();
        let n = raw.iter().find(|e| e.id == "new").unwrap();
        assert_eq!((n.remote_id.as_deref(), n.dirty), (Some("rid1"), false));
        assert!(raw.iter().find(|e| e.id == "edited").unwrap().dirty);

        // 삭제 표시는 서버 삭제가 끝나면 목록에서 사라진다.
        let mut del = snapshot_new.clone();
        del.deleted = true;
        s.mutate_events(|l| l.iter_mut().find(|e| e.id == "new").unwrap().deleted = true).unwrap();
        del.updated_at = s.raw_events().into_iter().find(|e| e.id == "new").unwrap().updated_at;
        mark_pushed(&s, &del, None).unwrap();
        assert!(s.raw_events().iter().all(|e| e.id != "new"));
    }

    #[test]
    fn percent_encoding_roundtrip() {
        let s = "4/0AfJohXn=abc&x y+z가나";
        assert_eq!(percent_decode(&urlenc(s)), s);
        assert_eq!(percent_decode("a%2Fb%3D"), "a/b=");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}

/// 가짜 서버(WC_MOCK=http://127.0.0.1:8770)에 실제로 통신해 보는 테스트.
/// 환경 변수가 없으면 건너뛴다. (일반 `cargo test`에는 영향 없음)
#[cfg(test)]
mod net_tests {
    use super::*;
    use crate::store::Account;
    use serde_json::Value;

    fn mock() -> Option<String> {
        std::env::var("WC_MOCK").ok()
    }

    async fn post(base: &str, path: &str, body: &str) {
        reqwest::Client::new().post(format!("{base}/admin/{path}")).body(body.to_string()).send().await.unwrap();
    }

    async fn state(base: &str) -> Value {
        reqwest::get(format!("{base}/admin/state")).await.unwrap().json().await.unwrap()
    }

    fn store() -> Store {
        Store::load(std::env::temp_dir().join(format!("wc_net_{}", uuid::Uuid::new_v4().simple())))
    }

    fn ics(uid: &str, title: &str, extra: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:{title}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    fn day(n: i64) -> String {
        fmt_date(Local::now().date_naive() + Duration::days(n))
    }

    fn local_event(id: &str, title: &str, cal: &str, date: &str, time: Option<&str>) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start_date: date.into(),
            end_date: date.into(),
            all_day: time.is_none(),
            start_time: time.map(|t| t.to_string()),
            end_time: time.map(|_| "23:00".to_string()),
            reminders: vec![10],
            calendar_id: cal.into(),
            color: "#5b9dff".into(),
            ..Default::default()
        }
    }

    #[test]
    fn caldav_full_roundtrip() {
        let Some(b) = mock() else { return };
        tauri::async_runtime::block_on(async move {
            let s = store();
            let dt = day(3).replace('-', "");
            // 잘못된 비밀번호
            s.add_account(Account { id: "bad".into(), kind: "caldav".into(), url: format!("{b}/dav"), username: "u".into(), password: "x".into(), enabled: true, ..Default::default() }).unwrap();
            let e = sync_store(&s, "bad").await.unwrap_err();
            assert!(e.contains("인증에 실패"), "{e}");

            // 301 리다이렉트로 시작하는 주소 → 자동으로 캘린더 찾기(할 일 캘린더는 제외)
            s.add_account(Account { id: "dav".into(), kind: "caldav".into(), url: format!("{b}/old"), username: "u".into(), password: "p".into(), color: "#3ec99a".into(), enabled: true, ..Default::default() }).unwrap();
            post(&b, "dav_put/a.ics", &ics("a", "서버 일정", &format!("DTSTART;VALUE=DATE:{dt}\r\n"))).await;
            sync_store(&s, "dav").await.unwrap();
            let acc = s.account("dav").unwrap();
            assert_eq!(acc.calendars, vec![format!("{b}/dav/calendars/u/default/")]);
            let evs = s.events();
            assert_eq!(evs.len(), 1);
            assert_eq!(evs[0].title, "서버 일정");
            assert_eq!(evs[0].remote_id.as_deref(), Some(format!("{b}/dav/calendars/u/default/a.ics").as_str()));
            assert!(!evs[0].read_only && !evs[0].dirty);
            let server_id = evs[0].id.clone();

            // 로컬에서 만든 일정 → 서버에 올라감(알림 포함)
            s.upsert(local_event("n1", "로컬에서 만든 일정", "dav", &day(4), Some("14:00"))).unwrap();
            assert!(s.events().iter().find(|e| e.id == "n1").unwrap().dirty);
            sync_store(&s, "dav").await.unwrap();
            let st = state(&b).await;
            let uploaded: Vec<&str> = st["dav"].as_object().unwrap().values().map(|v| v.as_str().unwrap()).filter(|t| t.contains("로컬에서 만든 일정")).collect();
            assert_eq!(uploaded.len(), 1);
            assert!(uploaded[0].contains("TRIGGER:-PT10M") && uploaded[0].contains("DTSTART:"));
            let n1 = s.events().into_iter().find(|e| e.id == "n1").unwrap();
            assert!(n1.remote_id.is_some() && !n1.dirty);
            assert_eq!(s.events().len(), 2, "서버에서 받은 사본과 중복되면 안 된다");

            // 로컬 수정 → 서버 본문이 바뀜
            let mut edited = s.events().into_iter().find(|e| e.id == server_id).unwrap();
            edited.title = "서버 일정(수정됨)".into();
            s.upsert(edited).unwrap();
            sync_store(&s, "dav").await.unwrap();
            assert!(state(&b).await["dav"]["a.ics"].as_str().unwrap().contains("서버 일정(수정됨)"));

            // 로컬 삭제 → 서버에서도 삭제, 삭제 표시는 정리됨
            s.remove(&server_id).unwrap();
            sync_store(&s, "dav").await.unwrap();
            assert!(state(&b).await["dav"].get("a.ics").is_none());
            assert!(s.raw_events().iter().all(|e| e.id != server_id));

            // 서버에서 반복 일정 추가 → 읽기 전용 3건, 서버에서 지우면 사라짐
            post(&b, "dav_put/rep.ics", &ics("rep", "반복", &format!("DTSTART;VALUE=DATE:{dt}\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\n"))).await;
            sync_store(&s, "dav").await.unwrap();
            let reps: Vec<_> = s.events().into_iter().filter(|e| e.title == "반복").collect();
            assert_eq!(reps.len(), 3);
            assert!(reps.iter().all(|e| e.read_only));
            post(&b, "dav_delete/rep.ics", "").await;
            sync_store(&s, "dav").await.unwrap();
            assert!(s.events().iter().all(|e| e.title != "반복"));
            assert_eq!(s.account("dav").unwrap().last_error, "");
        });
    }

    #[test]
    fn google_full_roundtrip() {
        let Some(b) = mock() else { return };
        std::env::set_var("WC_GOOGLE_API", format!("{b}/google/events"));
        std::env::set_var("WC_GOOGLE_TOKEN", format!("{b}/google/token"));
        tauri::async_runtime::block_on(async move {
            let s = store();
            // 로그인이 만료된 계정
            s.add_account(Account { id: "gbad".into(), kind: "google".into(), client_id: "cid".into(), client_secret: "cs".into(), refresh_token: "nope".into(), enabled: true, ..Default::default() }).unwrap();
            let e = sync_store(&s, "gbad").await.unwrap_err();
            assert!(e.contains("만료"), "{e}");

            s.add_account(Account { id: "g".into(), kind: "google".into(), client_id: "cid".into(), client_secret: "cs".into(), refresh_token: "rt-ok".into(), color: "#3ec99a".into(), enabled: true, ..Default::default() }).unwrap();
            // 서버에 3건(2건씩 나눠서 내려옴): 시간 일정(+09:00), 종일 일정(3일, 종료일 미포함), 취소된 일정
            let (d1, d2, d3) = (day(2), day(5), day(9));
            post(&b, "g_add", &format!(r#"{{"id":"t1","summary":"시간 일정","start":{{"dateTime":"{d1}T14:00:00+09:00"}},"end":{{"dateTime":"{d1}T15:30:00+09:00"}},"reminders":{{"useDefault":false,"overrides":[{{"method":"popup","minutes":10}}]}}}}"#)).await;
            post(&b, "g_add", &format!(r#"{{"id":"a1","summary":"종일 일정","start":{{"date":"{d2}"}},"end":{{"date":"{}"}}}}"#, day(8))).await;
            post(&b, "g_add", &format!(r#"{{"id":"c1","status":"cancelled","summary":"취소됨","start":{{"date":"{d3}"}},"end":{{"date":"{d3}"}}}}"#)).await;
            sync_store(&s, "g").await.unwrap();

            let evs = s.events();
            assert_eq!(evs.len(), 2, "취소된 일정은 제외: {evs:?}");
            let t = evs.iter().find(|e| e.title == "시간 일정").unwrap();
            let expect = chrono::DateTime::parse_from_rfc3339(&format!("{d1}T14:00:00+09:00")).unwrap().with_timezone(&Local);
            assert_eq!(t.start_time.as_deref(), Some(expect.format("%H:%M").to_string().as_str()));
            assert_eq!(t.reminders, vec![10]);
            let a = evs.iter().find(|e| e.title == "종일 일정").unwrap();
            assert!(a.all_day);
            assert_eq!((a.start_date.clone(), a.end_date.clone()), (d2.clone(), day(7)));

            // 로컬에서 만든 시간·종일 일정이 올라감
            s.upsert(local_event("n1", "내가 만든 시간 일정", "g", &day(3), Some("09:30"))).unwrap();
            let mut allday = local_event("n2", "내가 만든 종일 일정", "g", &day(6), None);
            allday.end_date = day(7);
            allday.reminders = vec![0, 1440];
            s.upsert(allday).unwrap();
            sync_store(&s, "g").await.unwrap();
            let st = state(&b).await;
            let g = st["google"].as_object().unwrap();
            let timed = g.values().find(|v| v["summary"] == "내가 만든 시간 일정").unwrap();
            assert_eq!(timed["reminders"]["overrides"][0]["minutes"], 10);
            assert!(timed["start"]["dateTime"].as_str().unwrap().contains(&day(3)));
            let ad = g.values().find(|v| v["summary"] == "내가 만든 종일 일정").unwrap();
            assert_eq!(ad["start"]["date"], day(6));
            assert_eq!(ad["end"]["date"], day(8), "종일 일정의 종료일은 다음 날로 보낸다");
            assert_eq!(ad["reminders"]["overrides"][0]["minutes"], 540);   // 정시(09:00) = 자정 기준 540분 전
            assert_eq!(ad["reminders"]["overrides"][1]["minutes"], 1980);  // 1일 전 09:00
            assert!(ad["start"].get("dateTime").is_none());
            assert_eq!(s.events().len(), 4, "올린 일정이 서버 사본과 중복되면 안 된다");
            assert!(s.events().iter().all(|e| !e.dirty));

            // 로컬 수정 → PATCH
            let mut e1 = s.events().into_iter().find(|e| e.id == "n1").unwrap();
            e1.title = "제목 바뀜".into();
            s.upsert(e1).unwrap();
            sync_store(&s, "g").await.unwrap();
            assert!(state(&b).await["google"].as_object().unwrap().values().any(|v| v["summary"] == "제목 바뀜"));

            // 로컬 삭제 → 서버 삭제
            s.remove("n1").unwrap();
            sync_store(&s, "g").await.unwrap();
            assert!(!state(&b).await["google"].as_object().unwrap().values().any(|v| v["summary"] == "제목 바뀜"));

            // 서버에서 지워진 일정은 로컬에서도 사라짐
            post(&b, "g_del/t1", "").await;
            sync_store(&s, "g").await.unwrap();
            assert!(s.events().iter().all(|e| e.title != "시간 일정"));
            assert_eq!(s.account("g").unwrap().last_error, "");
        });
    }

    /// 브라우저 대신 쓰는 아주 단순한 HTTP 요청
    fn raw_http(addr: &str, req: &str) -> String {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    fn query_of(url: &str) -> std::collections::HashMap<String, String> {
        reqwest::Url::parse(url).unwrap().query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn google_login_flow_with_pkce_and_denial() {
        let Some(b) = mock() else { return };
        std::env::set_var("WC_GOOGLE_TOKEN", format!("{b}/google/token"));
        let host = b.trim_start_matches("http://").to_string();
        tauri::async_runtime::block_on(async move {
            // 정상 로그인: 로그인 주소 검사 → (잡음 요청은 무시) → 올바른 state와 code로 완료
            let h = host.clone();
            let refresh = google::authorize_with("cid", "cs", move |url: &str| {
                let q = query_of(url);
                assert_eq!(q["client_id"], "cid");
                assert_eq!(q["code_challenge_method"], "S256");
                assert_eq!(q["access_type"], "offline");
                assert!(q["scope"].contains("calendar"));
                let port = reqwest::Url::parse(&q["redirect_uri"]).unwrap().port().unwrap();
                let addr = format!("127.0.0.1:{port}");
                let (state, chal) = (q["state"].clone(), q["code_challenge"].clone());
                std::thread::spawn(move || {
                    raw_http(&h, &format!("POST /admin/g_challenge HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", chal.len(), chal));
                    raw_http(&addr, "GET /favicon.ico HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
                    raw_http(&addr, "GET /?code=evil&state=WRONG HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
                    let page = raw_http(&addr, &format!("GET /?code=abc&state={state} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
                    assert!(page.contains("로그인이 완료"), "{page}");
                });
            })
            .await
            .unwrap();
            assert_eq!(refresh, "rt-ok");

            // 사용자가 구글 화면에서 "취소"를 누른 경우
            let err = google::authorize_with("cid", "cs", |url: &str| {
                let q = query_of(url);
                let port = reqwest::Url::parse(&q["redirect_uri"]).unwrap().port().unwrap();
                let (addr, state) = (format!("127.0.0.1:{port}"), q["state"].clone());
                std::thread::spawn(move || {
                    raw_http(&addr, &format!("GET /?error=access_denied&state={state} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
                });
            })
            .await
            .unwrap_err();
            assert!(err.contains("취소"), "{err}");

            // 코드 교환이 거부되면(잘못된 Client Secret) 구글의 안내 문구를 그대로 전달한다.
            let err = google::authorize_with("cid", "WRONG", |url: &str| {
                let q = query_of(url);
                let port = reqwest::Url::parse(&q["redirect_uri"]).unwrap().port().unwrap();
                let (addr, state) = (format!("127.0.0.1:{port}"), q["state"].clone());
                std::thread::spawn(move || {
                    raw_http(&addr, &format!("GET /?code=abc&state={state} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"));
                });
            })
            .await
            .unwrap_err();
            assert!(err.contains("bad code"), "{err}");
        });
    }

    #[test]
    fn google_account_without_own_credentials_uses_builtin_file() {
        let Some(b) = mock() else { return };
        std::env::set_var("WC_GOOGLE_API", format!("{b}/google/events"));
        std::env::set_var("WC_GOOGLE_TOKEN", format!("{b}/google/token"));
        tauri::async_runtime::block_on(async move {
            let s = store();
            // 내장 정보가 없으면 로그인 정보가 없다고 안내한다(저장소의 google_client.json은 비어 있음).
            assert!(google::builtin_client(s.dir()).is_none());
            s.add_account(Account { id: "g".into(), kind: "google".into(), refresh_token: "rt-ok".into(), color: "#3ec99a".into(), enabled: true, ..Default::default() }).unwrap();
            let e = sync_store(&s, "g").await.unwrap_err();
            assert!(e.contains("Client ID"), "{e}");

            // 데이터 폴더에 google_client.json을 두면 다시 빌드하지 않고도 동기화된다.
            std::fs::write(s.dir().join("google_client.json"), r#"{"clientId":"cid","clientSecret":"cs"}"#).unwrap();
            assert!(google::builtin_client(s.dir()).is_some());
            sync_store(&s, "g").await.unwrap();
            assert_eq!(s.account("g").unwrap().last_error, "");
        });
    }
}
