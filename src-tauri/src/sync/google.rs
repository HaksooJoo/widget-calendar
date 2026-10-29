//! Google Calendar 양방향 동기화 (기본 캘린더).
//!
//! 로그인은 "데스크톱 앱" OAuth(루프백 + PKCE)를 쓴다. 사용자가 Google Cloud Console에서 만든
//! 클라이언트 ID/보안 비밀을 입력하면, 브라우저에서 로그인한 뒤 받은 refresh token을 저장한다.

use super::{fmt_date, http_client, ical, mark_pushed, open_url, pending, percent_decode, reconcile, urlenc, window};
use crate::store::{Account, Event, Store};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    time::{Duration as StdDuration, Instant},
};
use uuid::Uuid;

/// 빌드할 때 포함되는 Google 로그인 정보(src-tauri/google_client.json). 비어 있으면 사용자가 직접 입력해야 한다.
const EMBEDDED_CLIENT: &str = include_str!("../../google_client.json");

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ClientFile {
    client_id: String,
    client_secret: String,
}

fn parse_client(text: &str) -> Option<(String, String)> {
    let f: ClientFile = serde_json::from_str(text).ok()?;
    let (id, secret) = (f.client_id.trim().to_string(), f.client_secret.trim().to_string());
    if id.is_empty() || secret.is_empty() {
        None
    } else {
        Some((id, secret))
    }
}

/// 앱에 내장된(또는 데이터 폴더의 google_client.json로 덮어쓴) Google 로그인 정보.
/// 데이터 폴더의 파일이 있으면 그것을 우선해서, 다시 빌드하지 않고도 바꿀 수 있다.
pub fn builtin_client(data_dir: &Path) -> Option<(String, String)> {
    std::fs::read_to_string(data_dir.join("google_client.json"))
        .ok()
        .and_then(|t| parse_client(&t))
        .or_else(|| parse_client(EMBEDDED_CLIENT))
}

/// 계정이 직접 입력한 값을 갖고 있으면 그것을, 아니면 내장 값을 쓴다.
fn creds(store: &Store, acc: &Account) -> Result<(String, String), String> {
    if !acc.client_id.is_empty() {
        return Ok((acc.client_id.clone(), acc.client_secret.clone()));
    }
    builtin_client(store.dir()).ok_or_else(|| {
        "Google 로그인 정보(Client ID)가 이 앱에 설정되어 있지 않습니다. README의 'Google 로그인 설정'을 확인하세요.".to_string()
    })
}

fn api_base() -> String {
    std::env::var("WC_GOOGLE_API")
        .unwrap_or_else(|_| "https://www.googleapis.com/calendar/v3/calendars/primary/events".into())
}

fn token_url() -> String {
    std::env::var("WC_GOOGLE_TOKEN").unwrap_or_else(|_| "https://oauth2.googleapis.com/token".into())
}

/// Google이 돌려준 오류를 바로 조치할 수 있는 한국어 안내로 바꾼다. 원문도 괄호로 덧붙인다.
fn google_error(v: &Value, fallback: &str) -> String {
    let raw = v["error_description"]
        .as_str()
        .or_else(|| v["error"]["message"].as_str())
        .or_else(|| v["error"].as_str())
        .unwrap_or("")
        .to_string();
    let code = v["error"].as_str().unwrap_or("");
    let status = v["error"]["status"].as_str().unwrap_or("");
    let mut reasons = String::new();
    for key in ["errors", "details"] {
        if let Some(arr) = v["error"][key].as_array() {
            for e in arr {
                if let Some(r) = e["reason"].as_str() {
                    reasons.push_str(r);
                    reasons.push(' ');
                }
            }
        }
    }
    let low = format!("{raw} {code} {status} {reasons}").to_lowercase();
    let hint = if low.contains("invalid_client") || low.contains("client_secret") || low.contains("oauth client was not found") {
        Some("Client ID 또는 Client Secret이 올바르지 않습니다. Google Cloud Console에서 '데스크톱 앱' 유형으로 만든 값을 그대로 복사했는지 확인해 주세요.")
    } else if low.contains("redirect_uri_mismatch") {
        Some("OAuth 클라이언트 유형이 '데스크톱 앱'이 아닙니다. Google Cloud Console에서 '데스크톱 앱' 유형으로 새로 만들어 주세요.")
    } else if low.contains("invalid_grant") {
        Some("로그인이 만료되었거나 취소되었습니다. 계정을 삭제하고 다시 로그인해 주세요. (OAuth 동의 화면이 '테스트' 상태이면 7일마다 만료됩니다)")
    } else if low.contains("accessnotconfigured") || low.contains("service_disabled") || low.contains("has not been used in project") || low.contains("api has not been") {
        Some("Google Calendar API가 사용 설정되어 있지 않습니다. Cloud Console → API 및 서비스 → 라이브러리에서 'Google Calendar API'를 사용 설정해 주세요.")
    } else if low.contains("insufficient") || low.contains("invalid_scope") {
        Some("캘린더 권한이 허용되지 않았습니다. 다시 로그인하면서 권한 화면의 Google 캘린더 항목을 체크해 주세요.")
    } else if low.contains("access_denied") {
        Some("Google이 접근을 거부했습니다. 동의 화면이 '테스트' 상태라면 로그인하는 계정을 '테스트 사용자'로 등록해야 합니다.")
    } else if low.contains("ratelimit") || low.contains("quota") {
        Some("Google 요청 한도를 넘었습니다. 잠시 후 다시 시도해 주세요.")
    } else if low.contains("unauthenticated") || low.contains("invalid credentials") {
        Some("로그인 정보가 유효하지 않습니다. 계정을 삭제하고 다시 로그인해 주세요.")
    } else {
        None
    };
    match (hint, raw.is_empty()) {
        (Some(h), false) => format!("{h} (원문: {raw})"),
        (Some(h), true) => h.to_string(),
        (None, false) => raw,
        (None, true) => fallback.to_string(),
    }
}

// ───────────────────────── 로그인 ─────────────────────────

const SUCCESS_PAGE: &str = "<!doctype html><html lang=\"ko\"><meta charset=\"utf-8\"><title>Widget Calendar</title><body style=\"font-family:'Segoe UI','Malgun Gothic',sans-serif;background:#1a1d26;color:#e8ebf3;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h2>로그인이 완료되었습니다</h2><p style=\"color:#8b93a7\">이 창은 닫아도 됩니다. Widget Calendar로 돌아가세요.</p></div></body></html>";
const FAIL_PAGE: &str = "<!doctype html><html lang=\"ko\"><meta charset=\"utf-8\"><title>Widget Calendar</title><body style=\"font-family:'Segoe UI','Malgun Gothic',sans-serif;background:#1a1d26;color:#e8ebf3;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h2>로그인에 실패했습니다</h2><p style=\"color:#8b93a7\">Widget Calendar로 돌아가 다시 시도하세요.</p></div></body></html>";

fn respond(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    let msg = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    );
    let _ = stream.write_all(msg.as_bytes());
}

/// 브라우저가 돌아오는 주소(127.0.0.1)로 들어온 요청에서 인증 코드를 꺼낸다.
fn wait_for_code(listener: TcpListener, state: &str) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + StdDuration::from_secs(180);
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(StdDuration::from_secs(5)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
                let query = path.split_once('?').map(|x| x.1).unwrap_or("");
                let mut code = None;
                let mut st = None;
                let mut err = None;
                for kv in query.split('&') {
                    if let Some((k, v)) = kv.split_once('=') {
                        match k {
                            "code" => code = Some(percent_decode(v)),
                            "state" => st = Some(percent_decode(v)),
                            "error" => err = Some(percent_decode(v)),
                            _ => {}
                        }
                    }
                }
                if let Some(e) = err {
                    respond(&mut stream, "200 OK", FAIL_PAGE);
                    return Err(if e == "access_denied" {
                        "로그인이 취소되었습니다.".to_string()
                    } else {
                        format!("로그인에 실패했습니다: {e}")
                    });
                }
                match (code, st) {
                    (Some(c), Some(s)) if s == state => {
                        respond(&mut stream, "200 OK", SUCCESS_PAGE);
                        return Ok(c);
                    }
                    _ => respond(&mut stream, "404 Not Found", ""), // favicon 등
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() > deadline {
                    return Err("로그인 시간이 초과되었습니다(3분). 다시 시도해 주세요.".into());
                }
                std::thread::sleep(StdDuration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// 기본 브라우저로 Google 로그인을 진행하고 refresh token을 돌려준다.
pub async fn authorize(client_id: &str, client_secret: &str) -> Result<String, String> {
    authorize_with(client_id, client_secret, open_url).await
}

/// `open`이 로그인 주소를 여는 방법이다(테스트에서는 브라우저 대신 흉내 낸다).
pub async fn authorize_with(client_id: &str, client_secret: &str, open: impl FnOnce(&str)) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("로그인 준비에 실패했습니다: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = Uuid::new_v4().simple().to_string();

    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}&code_challenge={}&code_challenge_method=S256",
        urlenc(client_id),
        urlenc(&redirect),
        urlenc("https://www.googleapis.com/auth/calendar"),
        state,
        challenge
    );
    open(&url);

    let st = state.clone();
    let code = tauri::async_runtime::spawn_blocking(move || wait_for_code(listener, &st))
        .await
        .map_err(|e| e.to_string())??;

    let client = http_client(false)?;
    let resp = client
        .post(token_url())
        .form(&[
            ("code", code.as_str()),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Google 서버에 연결할 수 없습니다. 인터넷 연결·프록시·방화벽을 확인해 주세요. ({e})"))?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    // 새 Google 동의 화면은 항목별로 체크할 수 있어서, 캘린더 항목을 빼면 로그인은 되어도 일정을 못 가져온다.
    if let Some(sc) = v["scope"].as_str() {
        if !sc.split_whitespace().any(|s| s == "https://www.googleapis.com/auth/calendar") {
            return Err("캘린더 권한이 허용되지 않았습니다. 다시 로그인하면서 권한 화면의 'Google 캘린더' 항목을 체크해 주세요.".into());
        }
    }
    match v["refresh_token"].as_str() {
        Some(t) => Ok(t.to_string()),
        None => Err(google_error(&v, "refresh token을 받지 못했습니다. Client ID와 보안 비밀을 확인해 주세요.")),
    }
}

async fn access_token(client: &reqwest::Client, client_id: &str, client_secret: &str, refresh_token: &str) -> Result<String, String> {
    let resp = client
        .post(token_url())
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("Google 서버에 연결할 수 없습니다. 인터넷 연결·프록시·방화벽을 확인해 주세요. ({e})"))?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    match v["access_token"].as_str() {
        Some(t) => Ok(t.to_string()),
        None => {
            let msg = google_error(&v, "토큰 갱신에 실패했습니다.");
            if v["error"].as_str() == Some("invalid_grant") {
                Err("Google 로그인이 만료되었습니다. 계정을 삭제하고 다시 연결해 주세요.".into())
            } else {
                Err(msg)
            }
        }
    }
}

// ───────────────────────── 변환 ─────────────────────────

fn local_rfc3339(ndt: NaiveDateTime) -> String {
    match Local.from_local_datetime(&ndt).earliest() {
        Some(dt) => dt.to_rfc3339_opts(SecondsFormat::Secs, false),
        None => Utc.from_utc_datetime(&ndt).to_rfc3339_opts(SecondsFormat::Secs, true),
    }
}

fn to_google_body(ev: &Event) -> Value {
    let all_day = ev.all_day || ev.start_time.is_none();
    let overrides: Vec<Value> = ev
        .reminders
        .iter()
        .take(5) // Google은 알림을 일정당 최대 5개까지만 받는다(초과하면 400 오류)
        .map(|m| {
            json!({
                "method": "popup",
                "minutes": if all_day { m + ical::ALL_DAY_OFFSET_MIN } else { *m }
            })
        })
        .collect();
    let mut v = json!({
        "summary": ev.title,
        "description": ev.memo,
        "reminders": { "useDefault": false, "overrides": overrides },
    });
    if let Some((s, e)) = ical::event_span(ev) {
        if all_day {
            v["start"] = json!({ "date": s.format("%Y-%m-%d").to_string(), "dateTime": null });
            v["end"] = json!({ "date": e.format("%Y-%m-%d").to_string(), "dateTime": null });
        } else {
            v["start"] = json!({ "dateTime": local_rfc3339(s), "date": null });
            v["end"] = json!({ "dateTime": local_rfc3339(e), "date": null });
        }
    }
    v
}

fn parse_gtime(v: &Value) -> Option<(NaiveDate, Option<NaiveTime>)> {
    if let Some(d) = v["date"].as_str() {
        return NaiveDate::parse_from_str(d, "%Y-%m-%d").ok().map(|d| (d, None));
    }
    let dt = DateTime::parse_from_rfc3339(v["dateTime"].as_str()?).ok()?;
    let local = dt.with_timezone(&Local).naive_local();
    Some((local.date(), Some(local.time())))
}

fn from_google(item: &Value, acc: &Account) -> Option<Event> {
    if item["status"].as_str() == Some("cancelled") {
        return None;
    }
    let id = item["id"].as_str()?.to_string();
    let (sd, st) = parse_gtime(&item["start"])?;
    let all_day = st.is_none();
    let (ed, et) = match parse_gtime(&item["end"]) {
        Some((d, None)) => ((d - Duration::days(1)).max(sd), None), // 종일 일정의 종료일은 포함하지 않는 값
        Some((d, Some(t))) => (d, Some(t)),
        None => (sd, st),
    };

    let mut reminders: Vec<i32> = Vec::new();
    if item["reminders"]["useDefault"].as_bool() == Some(false) {
        if let Some(arr) = item["reminders"]["overrides"].as_array() {
            for o in arr {
                if let Some(m) = o["minutes"].as_i64() {
                    let mut m = m as i32;
                    if all_day {
                        m -= ical::ALL_DAY_OFFSET_MIN;
                    }
                    if m >= 0 && !reminders.contains(&m) {
                        reminders.push(m);
                    }
                }
            }
        }
    }
    reminders.sort();

    let updated_at = item["updated"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);

    // 생일·집중 시간 같은 특수 일정은 수정할 수 없게 한다.
    let kind = item["eventType"].as_str().unwrap_or("default");

    Some(Event {
        id: String::new(),
        title: match item["summary"].as_str() {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => "(제목 없음)".to_string(),
        },
        start_date: fmt_date(sd),
        end_date: fmt_date(ed),
        all_day,
        start_time: st.map(|t| t.format("%H:%M").to_string()),
        end_time: et.map(|t| t.format("%H:%M").to_string()),
        memo: item["description"].as_str().unwrap_or("").to_string(),
        color: acc.color.clone(),
        reminders,
        calendar_id: acc.id.clone(),
        remote_id: Some(id),
        updated_at,
        deleted: false,
        read_only: kind != "default",
        dirty: false,
    })
}

// ───────────────────────── 동기화 ─────────────────────────

fn rfc3339_utc(d: NaiveDate) -> String {
    let ndt = d.and_time(NaiveTime::MIN);
    Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|l| l.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&ndt))
        .to_rfc3339_opts(SecondsFormat::Secs, true)
}

async fn pull(client: &reqwest::Client, tok: &str, acc: &Account) -> Result<Vec<Event>, String> {
    let (ws, we) = window();
    let (tmin, tmax) = (rfc3339_utc(ws), rfc3339_utc(we));
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..50 {
        let mut q: Vec<(&str, String)> = vec![
            ("singleEvents", "true".into()),
            ("orderBy", "startTime".into()),
            ("maxResults", "2500".into()),
            ("timeMin", tmin.clone()),
            ("timeMax", tmax.clone()),
        ];
        if let Some(p) = &page {
            q.push(("pageToken", p.clone()));
        }
        let resp = client
            .get(api_base())
            .bearer_auth(tok)
            .query(&q)
            .send()
            .await
            .map_err(|e| format!("Google 서버에 연결할 수 없습니다. 인터넷 연결·프록시·방화벽을 확인해 주세요. ({e})"))?;
        let status = resp.status();
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(google_error(&v, &format!("일정을 가져오지 못했습니다 ({}).", status.as_u16())));
        }
        if let Some(items) = v["items"].as_array() {
            out.extend(items.iter().filter_map(|i| from_google(i, acc)));
        }
        match v["nextPageToken"].as_str() {
            Some(t) => page = Some(t.to_string()),
            None => break,
        }
    }
    Ok(out)
}

pub async fn sync(store: &Store, acc: &Account) -> Result<(), String> {
    let client = http_client(false)?;
    let (cid, secret) = creds(store, acc)?;
    let tok = access_token(&client, &cid, &secret, &acc.refresh_token).await?;
    let mut errors: Vec<String> = Vec::new();

    // ① 이 PC의 변경을 서버에 올린다.
    for ev in pending(store, &acc.id) {
        let result: Result<Option<String>, String> = async {
            if ev.deleted {
                if let Some(rid) = &ev.remote_id {
                    let resp = client
                        .delete(format!("{}/{}", api_base(), urlenc(rid)))
                        .bearer_auth(&tok)
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    let s = resp.status().as_u16();
                    if !(resp.status().is_success() || s == 404 || s == 410) {
                        return Err(format!("삭제 실패 ({s})"));
                    }
                }
                Ok(None)
            } else if let Some(rid) = &ev.remote_id {
                let resp = client
                    .patch(format!("{}/{}", api_base(), urlenc(rid)))
                    .bearer_auth(&tok)
                    .json(&to_google_body(&ev))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = resp.status();
                if !status.is_success() {
                    let v: Value = resp.json().await.unwrap_or(Value::Null);
                    return Err(google_error(&v, &format!("수정 실패 ({})", status.as_u16())));
                }
                Ok(None)
            } else {
                let resp = client
                    .post(api_base())
                    .bearer_auth(&tok)
                    .json(&to_google_body(&ev))
                    .send()
                    .await
                    .map_err(|e| e.to_string())?;
                let status = resp.status();
                let v: Value = resp.json().await.map_err(|e| e.to_string())?;
                if !status.is_success() {
                    return Err(google_error(&v, &format!("등록 실패 ({})", status.as_u16())));
                }
                Ok(v["id"].as_str().map(|s| s.to_string()))
            }
        }
        .await;

        match result {
            Ok(rid) => mark_pushed(store, &ev, rid)?,
            Err(e) => errors.push(format!("'{}': {}", ev.title, e)),
        }
    }

    // ② 서버의 현재 상태를 받아 합친다.
    let remote = pull(&client, &tok, acc).await?;
    let (ws, we) = window();
    reconcile(store, &acc.id, remote, ws, we)?;

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("일부 변경을 서버에 올리지 못했습니다 — {}", errors.join(" / ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_file_parsing() {
        assert_eq!(parse_client(r#"{"clientId":" a ","clientSecret":"b"}"#), Some(("a".into(), "b".into())));
        assert_eq!(parse_client(r#"{"_안내":"x","clientId":"","clientSecret":""}"#), None);
        assert_eq!(parse_client(r#"{"clientId":"a"}"#), None);
        assert_eq!(parse_client("깨진 파일"), None);
    }
}
