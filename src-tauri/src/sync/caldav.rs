//! CalDAV 동기화(네이버, iCloud, Nextcloud 등 표준을 따르는 서버).
//! 서버 주소, 아이디, 비밀번호만 받아 캘린더를 자동으로 찾는다.

use super::{fmt_date, http_client, ical, mark_pushed, pending, reconcile, window};
use crate::store::{Account, Event, Store};
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc, Local};
use reqwest::Url;
use roxmltree::{Document, Node};
use uuid::Uuid;

pub struct CalInfo {
    pub url: String,
}

struct Dav {
    client: reqwest::Client,
    user: String,
    pass: String,
}

fn join(base: &str, href: &str) -> Result<String, String> {
    Url::parse(base)
        .and_then(|b| b.join(href))
        .map(|u| u.to_string())
        .map_err(|e| format!("주소가 올바르지 않습니다: {e}"))
}

fn status_error(status: u16) -> String {
    match status {
        401 => "인증에 실패했습니다. 아이디와 비밀번호를 확인해 주세요. (2단계 인증을 쓰면 '애플리케이션 비밀번호'가 필요합니다)".into(),
        403 => "서버가 접근을 거부했습니다(403).".into(),
        404 => "서버에서 해당 주소를 찾을 수 없습니다(404).".into(),
        s => format!("서버 오류({s})"),
    }
}

impl Dav {
    fn new(acc: &Account) -> Result<Dav, String> {
        Ok(Dav {
            client: http_client(true)?, // 리다이렉트는 메서드를 유지한 채 직접 따라간다.
            user: acc.username.clone(),
            pass: acc.password.clone(),
        })
    }

    async fn req(
        &self,
        method: &str,
        url: &str,
        depth: Option<&str>,
        content_type: &str,
        body: &str,
        extra: &[(&str, &str)],
    ) -> Result<(u16, String), String> {
        let mut url = url.to_string();
        for _ in 0..6 {
            let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
            let mut rb = self.client.request(m, &url).basic_auth(&self.user, Some(&self.pass));
            if !body.is_empty() {
                rb = rb.header("Content-Type", content_type).body(body.to_string());
            }
            if let Some(d) = depth {
                rb = rb.header("Depth", d);
            }
            for (k, v) in extra {
                rb = rb.header(*k, *v);
            }
            let resp = rb.send().await.map_err(|e| format!("서버에 연결할 수 없습니다: {e}"))?;
            let st = resp.status().as_u16();
            if matches!(st, 301 | 302 | 307 | 308) {
                if let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                    url = join(&url, loc)?;
                    continue;
                }
            }
            let text = resp.text().await.unwrap_or_default();
            return Ok((st, text));
        }
        Err("리다이렉트가 너무 많습니다.".into())
    }
}

fn child<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn desc<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.descendants().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text_of(n: Option<Node>) -> Option<String> {
    n.and_then(|n| n.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

const PROP_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
const PROP_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#;
const PROP_CALS: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><c:supported-calendar-component-set/></d:prop></d:propfind>"#;

/// PROPFIND로 href 하나를 찾는다. (current-user-principal / calendar-home-set)
async fn propfind_href(dav: &Dav, url: &str, body: &str, prop: &str) -> Result<Option<String>, String> {
    let (st, text) = dav.req("PROPFIND", url, Some("0"), "application/xml; charset=utf-8", body, &[]).await?;
    if st == 401 || st == 403 {
        return Err(status_error(st));
    }
    if st >= 400 {
        return Ok(None);
    }
    let doc = Document::parse(&text).map_err(|_| "서버 응답을 해석할 수 없습니다.".to_string())?;
    let found = desc(doc.root(), prop).and_then(|p| desc(p, "href")).and_then(|h| text_of(Some(h)));
    Ok(found)
}

fn parse_calendars(text: &str, base: &str) -> Result<Vec<CalInfo>, String> {
    let doc = Document::parse(text).map_err(|_| "서버 응답을 해석할 수 없습니다.".to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "response") {
        let Some(href) = text_of(child(resp, "href")) else { continue };
        let is_cal = desc(resp, "resourcetype").map_or(false, |rt| child(rt, "calendar").is_some());
        if !is_cal {
            continue;
        }
        // 지원하는 구성요소 목록이 있으면 VEVENT가 들어 있어야 한다(할 일 전용 캘린더 제외).
        if let Some(set) = desc(resp, "supported-calendar-component-set") {
            let has = set
                .children()
                .filter(|c| c.is_element())
                .any(|c| c.attribute("name").map_or(false, |n| n.eq_ignore_ascii_case("VEVENT")));
            if !has {
                continue;
            }
        }
        let mut url = join(base, &href)?;
        if !url.ends_with('/') {
            url.push('/');
        }
        out.push(CalInfo { url });
    }
    Ok(out)
}

/// 서버 주소와 계정으로 캘린더 목록을 찾는다.
pub async fn discover(acc: &Account) -> Result<Vec<CalInfo>, String> {
    let dav = Dav::new(acc)?;
    let base = acc.url.trim().trim_end_matches('/').to_string();

    // 시작점 후보: 입력한 주소 → well-known → (네이버) 사용자 주소
    let mut starts: Vec<String> = vec![format!("{base}/")];
    starts.push(format!("{base}/.well-known/caldav"));
    if base.contains("naver.com") {
        starts.push(format!("{base}/principals/users/{}/", acc.username));
    }

    let mut last_err: Option<String> = None;
    for start in starts {
        let attempt: Result<Vec<CalInfo>, String> = async {
            // 1) 사용자 주소(principal)
            let principal = match propfind_href(&dav, &start, PROP_PRINCIPAL, "current-user-principal").await? {
                Some(h) => join(&start, &h)?,
                None => start.clone(),
            };
            // 2) 캘린더 모음 주소(home set)
            let home = match propfind_href(&dav, &principal, PROP_HOME, "calendar-home-set").await? {
                Some(h) => join(&principal, &h)?,
                None => principal.clone(),
            };
            // 3) 캘린더 목록
            let (st, text) = dav
                .req("PROPFIND", &home, Some("1"), "application/xml; charset=utf-8", PROP_CALS, &[])
                .await?;
            if st == 401 || st == 403 {
                return Err(status_error(st));
            }
            if st >= 400 {
                return Err(status_error(st));
            }
            parse_calendars(&text, &home)
        }
        .await;

        match attempt {
            Ok(list) if !list.is_empty() => return Ok(list),
            Ok(_) => last_err = Some("이 계정에서 일정 캘린더를 찾지 못했습니다.".into()),
            Err(e) => {
                if e.starts_with("인증에 실패") {
                    return Err(e); // 비밀번호 문제는 다른 주소를 시도해도 같다.
                }
                last_err = Some(e)
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "캘린더를 찾지 못했습니다.".into()))
}

// ───────────────────────── 동기화 ─────────────────────────

fn dav_time(d: NaiveDate) -> String {
    let ndt = d.and_time(NaiveTime::MIN);
    Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|l| l.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&ndt))
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

async fn pull_calendar(dav: &Dav, cal: &str, acc: &Account, ws: NaiveDate, we: NaiveDate) -> Result<Vec<Event>, String> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{}" end="{}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
        dav_time(ws),
        dav_time(we)
    );
    let (st, text) = dav.req("REPORT", cal, Some("1"), "application/xml; charset=utf-8", &body, &[]).await?;
    if st >= 400 {
        return Err(status_error(st));
    }
    parse_report(&text, cal, acc, ws, we)
}

pub fn parse_report(text: &str, cal: &str, acc: &Account, ws: NaiveDate, we: NaiveDate) -> Result<Vec<Event>, String> {
    let doc = Document::parse(text).map_err(|_| "서버 응답을 해석할 수 없습니다.".to_string())?;
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "response") {
        let Some(href) = text_of(child(resp, "href")) else { continue };
        let Some(data) = desc(resp, "calendar-data").and_then(|n| n.text()) else { continue };
        let url = join(cal, &href)?;
        let parsed = ical::parse(data, ws, we);
        for p in parsed.items {
            let mut ev = p.event;
            // 반복 일정에서 펼친 항목이나 한 파일에 여러 일정이 든 경우는 안전하게 읽기 전용으로 둔다.
            let editable = !p.recurring && parsed.vevent_count == 1;
            ev.remote_id = Some(if editable { url.clone() } else { format!("{}#{}@{}", url, p.uid, ev.start_date) });
            ev.read_only = !editable;
            ev.color = acc.color.clone();
            ev.calendar_id = acc.id.clone();
            out.push(ev);
        }
    }
    Ok(out)
}

pub async fn sync(store: &Store, acc: &Account) -> Result<(), String> {
    let dav = Dav::new(acc)?;

    let mut cals = acc.calendars.clone();
    if cals.is_empty() {
        cals = discover(acc).await?.into_iter().map(|c| c.url).collect();
        let found = cals.clone();
        store.update_account(&acc.id, |a| a.calendars = found)?;
    }
    let target = cals.first().cloned().ok_or_else(|| "동기화할 캘린더가 없습니다.".to_string())?;
    let mut errors: Vec<String> = Vec::new();

    // ① 이 PC의 변경을 서버에 올린다.
    for ev in pending(store, &acc.id) {
        let result: Result<Option<String>, String> = async {
            if ev.deleted {
                if let Some(rid) = &ev.remote_id {
                    let (st, _) = dav.req("DELETE", rid, None, "", "", &[]).await?;
                    if !(st < 300 || st == 404 || st == 410) {
                        return Err(status_error(st));
                    }
                }
                Ok(None)
            } else if let Some(rid) = &ev.remote_id {
                let uid = rid.rsplit('/').next().unwrap_or("").trim_end_matches(".ics").to_string();
                let uid = if uid.is_empty() { Uuid::new_v4().to_string() } else { uid };
                let body = ical::build_resource(&ev, &uid);
                let (st, _) = dav.req("PUT", rid, None, "text/calendar; charset=utf-8", &body, &[]).await?;
                if st >= 300 {
                    return Err(status_error(st));
                }
                Ok(None)
            } else {
                let uid = Uuid::new_v4().to_string();
                let url = join(&target, &format!("{uid}.ics"))?;
                let body = ical::build_resource(&ev, &uid);
                let (st, _) = dav
                    .req("PUT", &url, None, "text/calendar; charset=utf-8", &body, &[("If-None-Match", "*")])
                    .await?;
                if st >= 300 {
                    return Err(status_error(st));
                }
                Ok(Some(url))
            }
        }
        .await;

        match result {
            Ok(rid) => mark_pushed(store, &ev, rid)?,
            Err(e) => errors.push(format!("'{}': {}", ev.title, e)),
        }
    }

    // ② 서버의 현재 상태를 받아 합친다.
    let (ws, we) = window();
    let mut remote: Vec<Event> = Vec::new();
    for cal in &cals {
        remote.extend(pull_calendar(&dav, cal, acc, ws, we).await?);
    }
    reconcile(store, &acc.id, remote, ws, we)?;

    let _ = fmt_date; // (공용 함수 import 유지)
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
    fn calendars_from_propfind() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
 <d:response><d:href>/cal/user/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>
 <d:response><d:href>/cal/user/default/</d:href><d:propstat><d:prop>
   <d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>내 캘린더</d:displayname>
   <c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
 <d:response><d:href>/cal/user/tasks/</d:href><d:propstat><d:prop>
   <d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>할 일</d:displayname>
   <c:supported-calendar-component-set><c:comp name="VTODO"/></c:supported-calendar-component-set></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let cals = parse_calendars(xml, "https://dav.example.com/cal/user/").unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].url, "https://dav.example.com/cal/user/default/");
    }

    #[test]
    fn report_to_events() {
        let xml = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
 <d:response><d:href>/cal/a.ics</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag>
  <c:calendar-data>BEGIN:VCALENDAR
BEGIN:VEVENT
UID:one
SUMMARY:단일 일정
DTSTART:20261028T140000
DTEND:20261028T150000
END:VEVENT
END:VCALENDAR
</c:calendar-data></d:prop></d:propstat></d:response>
 <d:response><d:href>/cal/b.ics</d:href><d:propstat><d:prop>
  <c:calendar-data><![CDATA[BEGIN:VCALENDAR
BEGIN:VEVENT
UID:rep
SUMMARY:반복
DTSTART;VALUE=DATE:20261005
RRULE:FREQ=WEEKLY;COUNT=3
END:VEVENT
END:VCALENDAR
]]></c:calendar-data></d:prop></d:propstat></d:response>
</d:multistatus>"#;
        let acc = Account { id: "acc1".into(), color: "#5b9dff".into(), ..Default::default() };
        let ws = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let we = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let evs = parse_report(xml, "https://dav.example.com/cal/", &acc, ws, we).unwrap();
        assert_eq!(evs.len(), 4);
        let one = evs.iter().find(|e| e.title == "단일 일정").unwrap();
        assert_eq!(one.remote_id.as_deref(), Some("https://dav.example.com/cal/a.ics"));
        assert!(!one.read_only);
        let reps: Vec<_> = evs.iter().filter(|e| e.title == "반복").collect();
        assert_eq!(reps.len(), 3);
        assert!(reps.iter().all(|e| e.read_only && e.remote_id.as_ref().unwrap().contains('#')));
    }
}
