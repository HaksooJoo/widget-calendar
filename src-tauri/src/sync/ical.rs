//! iCalendar(.ics) 읽기·쓰기. CalDAV, ICS 구독, 백업 내보내기가 함께 쓴다.
//! 반복 일정(RRULE)은 조회 구간 안의 날짜로 펼쳐서 개별 일정으로 돌려준다.

use crate::store::Event;
use chrono::{
    Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday,
};
use std::collections::HashSet;

/// 종일 일정의 알림 기준 시각(당일 09:00)과 iCal/Google 기준(자정)의 차이(분)
pub const ALL_DAY_OFFSET_MIN: i32 = 9 * 60;

pub struct Parsed {
    pub uid: String,
    pub event: Event,
    /// 반복 일정에서 나왔거나 반복 일정의 예외 항목이면 true (읽기 전용으로 다룬다)
    pub recurring: bool,
}

pub struct ParseResult {
    pub items: Vec<Parsed>,
    /// 파일 안의 VEVENT 개수
    pub vevent_count: usize,
}

#[derive(Default, Clone)]
struct Prop {
    value: String,
    tzid: Option<String>,
    date_only: bool,
}

#[derive(Default)]
struct Raw {
    uid: String,
    summary: String,
    description: String,
    dtstart: Option<Prop>,
    dtend: Option<Prop>,
    duration: Option<String>,
    rrule: Option<String>,
    exdates: Vec<Prop>,
    recurrence_id: Option<Prop>,
    cancelled: bool,
    last_modified: Option<Prop>,
    triggers: Vec<String>,
}

// ───────────────────────── 읽기 ─────────────────────────

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.trim_end_matches('\r');
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else {
            out.push(line.to_string());
        }
    }
    out
}

/// "NAME;K=V;K=V:value" → (NAME, [(K,V)], value). 따옴표 안의 ':' ';'는 구분자로 보지 않는다.
fn split_line(line: &str) -> Option<(String, Vec<(String, String)>, String)> {
    let mut in_q = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_q = !in_q,
            ':' if !in_q => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);

    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut q = false;
    for c in head.chars() {
        match c {
            '"' => {
                q = !q;
                cur.push(c)
            }
            ';' if !q => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);

    let name = parts.remove(0).to_uppercase();
    let params = parts
        .into_iter()
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_uppercase(), v.trim_matches('"').to_string()))
        })
        .collect();
    Some((name, params, value.to_string()))
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn make_prop(params: &[(String, String)], value: &str) -> Prop {
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    Prop {
        value: value.trim().to_string(),
        tzid: get("TZID"),
        date_only: get("VALUE").map(|v| v.eq_ignore_ascii_case("DATE")).unwrap_or(false),
    }
}

/// 날짜·시간을 이 PC의 로컬 시간으로 바꾼다. 시간이 없으면 종일.
fn to_local(p: &Prop) -> Option<(NaiveDate, Option<NaiveTime>)> {
    let v = p.value.trim();
    if p.date_only || v.len() == 8 {
        return NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d")
            .ok()
            .map(|d| (d, None));
    }
    let utc = v.ends_with('Z');
    let ndt = NaiveDateTime::parse_from_str(v.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    let local = if utc {
        Utc.from_utc_datetime(&ndt).with_timezone(&Local).naive_local()
    } else if let Some(tz) = p
        .tzid
        .as_deref()
        .map(|t| t.trim_matches('"').trim_start_matches('/'))
        .and_then(|t| t.parse::<chrono_tz::Tz>().ok())
    {
        match tz.from_local_datetime(&ndt).earliest() {
            Some(dt) => dt.with_timezone(&Local).naive_local(),
            None => ndt,
        }
    } else {
        ndt // 시간대 정보가 없으면 로컬 시간으로 간주
    };
    Some((local.date(), Some(local.time())))
}

/// "P1D", "PT1H30M", "-PT15M", "P1W" → 분
fn parse_duration_min(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            'W' | 'D' | 'H' | 'M' | 'S' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (c, in_time) {
                    ('W', _) => n * 7 * 1440,
                    ('D', _) => n * 1440,
                    ('H', _) => n * 60,
                    ('M', true) => n,
                    ('S', _) => n / 60,
                    _ => return None,
                };
            }
            _ => return None,
        }
    }
    Some(if neg { -total } else { total })
}

pub fn parse(text: &str, ws: NaiveDate, we: NaiveDate) -> ParseResult {
    let text = text.trim_start_matches('\u{feff}');
    let mut raws: Vec<Raw> = Vec::new();
    let mut cur: Option<Raw> = None;
    let mut in_alarm = false;

    for line in unfold(text) {
        let up = line.trim().to_uppercase();
        if up == "BEGIN:VEVENT" {
            cur = Some(Raw::default());
            in_alarm = false;
            continue;
        }
        if up == "END:VEVENT" {
            if let Some(r) = cur.take() {
                raws.push(r);
            }
            continue;
        }
        let Some(raw) = cur.as_mut() else { continue };
        if up == "BEGIN:VALARM" {
            in_alarm = true;
            continue;
        }
        if up == "END:VALARM" {
            in_alarm = false;
            continue;
        }
        let Some((name, params, value)) = split_line(&line) else { continue };
        if in_alarm {
            if name == "TRIGGER" {
                let related_end = params.iter().any(|(k, v)| k == "RELATED" && v == "END");
                let absolute = params.iter().any(|(k, v)| k == "VALUE" && v == "DATE-TIME");
                if !related_end && !absolute {
                    raw.triggers.push(value.trim().to_string());
                }
            }
            continue;
        }
        match name.as_str() {
            "UID" => raw.uid = value.trim().to_string(),
            "SUMMARY" => raw.summary = unescape(&value),
            "DESCRIPTION" => raw.description = unescape(&value),
            "DTSTART" => raw.dtstart = Some(make_prop(&params, &value)),
            "DTEND" => raw.dtend = Some(make_prop(&params, &value)),
            "DURATION" => raw.duration = Some(value.trim().to_string()),
            "RRULE" => raw.rrule = Some(value.trim().to_string()),
            "EXDATE" => {
                for part in value.split(',') {
                    raw.exdates.push(make_prop(&params, part));
                }
            }
            "RECURRENCE-ID" => raw.recurrence_id = Some(make_prop(&params, &value)),
            "STATUS" => raw.cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            "LAST-MODIFIED" => raw.last_modified = Some(make_prop(&params, &value)),
            _ => {}
        }
    }

    let vevent_count = raws.len();

    // 반복 일정의 예외 항목: (uid, 원래 날짜)
    let mut overrides: HashSet<(String, NaiveDate)> = HashSet::new();
    for r in &raws {
        if let Some(rid) = r.recurrence_id.as_ref().and_then(to_local) {
            overrides.insert((r.uid.clone(), rid.0));
        }
    }

    let mut items = Vec::new();
    for r in &raws {
        if r.cancelled {
            continue;
        }
        let Some(start) = r.dtstart.as_ref().and_then(to_local) else { continue };
        let all_day = start.1.is_none();
        let (start_date, start_time) = start;

        // 시작·종료의 간격을 구한다.
        let start_dt = start_date.and_time(start_time.unwrap_or(NaiveTime::MIN));
        let end_dt: NaiveDateTime = if let Some(end) = r.dtend.as_ref().and_then(to_local) {
            match end.1 {
                Some(t) => end.0.and_time(t),
                None => end.0.and_time(NaiveTime::MIN), // 종일: 종료일은 포함하지 않는 값
            }
        } else if let Some(min) = r.duration.as_deref().and_then(parse_duration_min) {
            start_dt + Duration::minutes(min)
        } else if all_day {
            start_dt + Duration::days(1)
        } else {
            start_dt
        };
        let span = (end_dt - start_dt).max(Duration::zero());

        let updated_at = r
            .last_modified
            .as_ref()
            .and_then(to_local)
            .map(|(d, t)| {
                d.and_time(t.unwrap_or(NaiveTime::MIN))
                    .and_local_timezone(Local)
                    .earliest()
                    .map(|x| x.timestamp_millis())
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        let reminders = reminders_from_triggers(&r.triggers, all_day);

        let make = |sd: NaiveDate| -> Event {
            let s = sd.and_time(start_time.unwrap_or(NaiveTime::MIN));
            let e = s + span;
            let (end_date, end_time) = if all_day {
                // 종일 일정의 DTEND는 다음 날 0시이므로 하루를 빼서 마지막 날로 맞춘다.
                let last = (e.date() - Duration::days(1)).max(sd);
                (last, None)
            } else {
                (e.date(), Some(e.time().format("%H:%M").to_string()))
            };
            Event {
                title: if r.summary.trim().is_empty() {
                    "(제목 없음)".to_string()
                } else {
                    r.summary.trim().to_string()
                },
                start_date: fmt_date(sd),
                end_date: fmt_date(end_date),
                all_day,
                start_time: start_time.map(|t| t.format("%H:%M").to_string()),
                end_time,
                memo: r.description.clone(),
                reminders: reminders.clone(),
                calendar_id: "local".to_string(),
                updated_at,
                ..Default::default()
            }
        };

        let overlaps = |ev: &Event| ev.start_date <= fmt_date(we) && ev.end_date >= fmt_date(ws);

        if r.recurrence_id.is_some() {
            // 반복 일정의 예외 항목
            let ev = make(start_date);
            if overlaps(&ev) {
                items.push(Parsed { uid: r.uid.clone(), event: ev, recurring: true });
            }
        } else if let Some(rule) = r.rrule.as_deref().and_then(parse_rrule) {
            let ex: HashSet<NaiveDate> =
                r.exdates.iter().filter_map(to_local).map(|(d, _)| d).collect();
            // 앞뒤로 span만큼 여유를 둬서 구간 가장자리에 걸친 일정을 놓치지 않는다.
            let pad = Duration::days(span.num_days() + 1);
            for d in expand(&rule, start_date, ws - pad, we) {
                if ex.contains(&d) || overrides.contains(&(r.uid.clone(), d)) {
                    continue;
                }
                let ev = make(d);
                if overlaps(&ev) {
                    items.push(Parsed { uid: r.uid.clone(), event: ev, recurring: true });
                }
            }
        } else {
            let ev = make(start_date);
            if overlaps(&ev) {
                items.push(Parsed { uid: r.uid.clone(), event: ev, recurring: false });
            }
        }
    }

    ParseResult { items, vevent_count }
}

fn fmt_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// TRIGGER(-PT15M 등)를 "몇 분 전" 목록으로. 종일 일정은 09:00 기준으로 환산한다.
fn reminders_from_triggers(triggers: &[String], all_day: bool) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for t in triggers {
        let Some(min) = parse_duration_min(t) else { continue };
        if min > 0 {
            continue; // 시작 이후에 울리는 알림은 지원하지 않음
        }
        let mut before = (-min) as i32;
        if all_day {
            before -= ALL_DAY_OFFSET_MIN;
            if before < 0 {
                continue;
            }
        }
        if !out.contains(&before) {
            out.push(before);
        }
    }
    out.sort();
    out
}

// ───────────────────────── 반복 규칙 ─────────────────────────

struct Rule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<NaiveDate>,
    byday: Vec<(i32, Weekday)>,
    bymonthday: Vec<i32>,
    bymonth: Vec<u32>,
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_rrule(s: &str) -> Option<Rule> {
    let mut rule = Rule {
        freq: String::new(),
        interval: 1,
        count: None,
        until: None,
        byday: vec![],
        bymonthday: vec![],
        bymonth: vec![],
    };
    for part in s.split(';') {
        let (k, v) = part.split_once('=')?;
        match k.to_uppercase().as_str() {
            "FREQ" => rule.freq = v.to_uppercase(),
            "INTERVAL" => rule.interval = v.parse::<i64>().ok()?.max(1),
            "COUNT" => rule.count = v.parse().ok(),
            "UNTIL" => {
                let day = v.get(..8)?;
                let d = NaiveDate::parse_from_str(day, "%Y%m%d").ok()?;
                // UTC로 적힌 값이면 로컬 날짜로 보정
                rule.until = Some(if v.ends_with('Z') && v.len() >= 15 {
                    to_local(&Prop { value: v.to_string(), ..Default::default() })
                        .map(|x| x.0)
                        .unwrap_or(d)
                } else {
                    d
                });
            }
            "BYDAY" => {
                for d in v.split(',') {
                    let d = d.trim();
                    if d.len() < 2 {
                        continue;
                    }
                    let (num, wd) = d.split_at(d.len() - 2);
                    let ord = if num.is_empty() { 0 } else { num.parse::<i32>().ok()? };
                    rule.byday.push((ord, parse_weekday(wd)?));
                }
            }
            "BYMONTHDAY" => rule.bymonthday = v.split(',').filter_map(|x| x.parse().ok()).collect(),
            "BYMONTH" => rule.bymonth = v.split(',').filter_map(|x| x.parse().ok()).collect(),
            _ => {}
        }
    }
    if ["DAILY", "WEEKLY", "MONTHLY", "YEARLY"].contains(&rule.freq.as_str()) {
        Some(rule)
    } else {
        None
    }
}

fn last_day_of_month(y: i32, m: u32) -> NaiveDate {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)
}

fn nth_weekday(y: i32, m: u32, ord: i32, wd: Weekday) -> Option<NaiveDate> {
    if ord > 0 {
        let first = NaiveDate::from_ymd_opt(y, m, 1)?;
        let shift = (wd.num_days_from_monday() as i64 - first.weekday().num_days_from_monday() as i64 + 7) % 7;
        let d = first + Duration::days(shift + 7 * (ord as i64 - 1));
        (d.month() == m).then_some(d)
    } else {
        let last = last_day_of_month(y, m);
        let shift = (last.weekday().num_days_from_monday() as i64 - wd.num_days_from_monday() as i64 + 7) % 7;
        let d = last - Duration::days(shift + 7 * ((-ord) as i64 - 1));
        (d.month() == m).then_some(d)
    }
}

struct Emit<'a> {
    rule: &'a Rule,
    start: NaiveDate,
    ws: NaiveDate,
    limit: NaiveDate,
    n: usize,
    out: Vec<NaiveDate>,
}

impl Emit<'_> {
    /// false를 돌려주면 더 만들 필요가 없다.
    fn push(&mut self, d: NaiveDate) -> bool {
        if d < self.start {
            return true;
        }
        if d > self.limit {
            return false;
        }
        self.n += 1;
        if let Some(c) = self.rule.count {
            if self.n > c {
                return false;
            }
        }
        if d >= self.ws {
            self.out.push(d);
        }
        true
    }
}

fn expand(rule: &Rule, start: NaiveDate, ws: NaiveDate, we: NaiveDate) -> Vec<NaiveDate> {
    let limit = rule.until.map_or(we, |u| u.min(we));
    let mut e = Emit { rule, start, ws, limit, n: 0, out: vec![] };

    match rule.freq.as_str() {
        "DAILY" => {
            let mut d = start;
            for _ in 0..30000 {
                if !e.push(d) {
                    break;
                }
                d += Duration::days(rule.interval);
            }
        }
        "WEEKLY" => {
            let days: Vec<Weekday> = if rule.byday.is_empty() {
                vec![start.weekday()]
            } else {
                rule.byday.iter().map(|(_, w)| *w).collect()
            };
            let week0 = start - Duration::days(start.weekday().num_days_from_monday() as i64);
            'outer: for wk in 0..6000i64 {
                let base = week0 + Duration::weeks(wk * rule.interval);
                if base > limit {
                    break;
                }
                for off in 0..7 {
                    let d = base + Duration::days(off);
                    if days.contains(&d.weekday()) && !e.push(d) {
                        break 'outer;
                    }
                }
            }
        }
        "MONTHLY" => {
            let (mut y, mut m) = (start.year(), start.month());
            'outer: for _ in 0..2400 {
                if NaiveDate::from_ymd_opt(y, m, 1).map_or(true, |f| f > limit) {
                    break;
                }
                let mut cands: Vec<NaiveDate> = Vec::new();
                if !rule.byday.is_empty() {
                    for (ord, wd) in &rule.byday {
                        if *ord == 0 {
                            let mut d = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
                            while d.month() == m {
                                if d.weekday() == *wd {
                                    cands.push(d);
                                }
                                d += Duration::days(1);
                            }
                        } else if let Some(d) = nth_weekday(y, m, *ord, *wd) {
                            cands.push(d);
                        }
                    }
                } else if !rule.bymonthday.is_empty() {
                    let last = last_day_of_month(y, m).day() as i32;
                    for md in &rule.bymonthday {
                        let day = if *md > 0 { *md } else { last + 1 + *md };
                        if (1..=last).contains(&day) {
                            cands.push(NaiveDate::from_ymd_opt(y, m, day as u32).unwrap());
                        }
                    }
                } else if let Some(d) = NaiveDate::from_ymd_opt(y, m, start.day()) {
                    cands.push(d);
                }
                cands.sort();
                cands.dedup();
                for d in cands {
                    if !e.push(d) {
                        break 'outer;
                    }
                }
                let total = (y as i64) * 12 + (m as i64 - 1) + rule.interval;
                y = (total / 12) as i32;
                m = (total % 12) as u32 + 1;
            }
        }
        "YEARLY" => {
            let months: Vec<u32> =
                if rule.bymonth.is_empty() { vec![start.month()] } else { rule.bymonth.clone() };
            let mut y = start.year();
            'outer: for _ in 0..400 {
                if NaiveDate::from_ymd_opt(y, 1, 1).map_or(true, |f| f > limit) {
                    break;
                }
                let mut cands: Vec<NaiveDate> = Vec::new();
                for &m in &months {
                    if !rule.byday.is_empty() {
                        for (ord, wd) in &rule.byday {
                            if *ord != 0 {
                                if let Some(d) = nth_weekday(y, m, *ord, *wd) {
                                    cands.push(d);
                                }
                            }
                        }
                    } else if !rule.bymonthday.is_empty() {
                        for md in &rule.bymonthday {
                            let last = last_day_of_month(y, m).day() as i32;
                            let day = if *md > 0 { *md } else { last + 1 + *md };
                            if (1..=last).contains(&day) {
                                cands.push(NaiveDate::from_ymd_opt(y, m, day as u32).unwrap());
                            }
                        }
                    } else if let Some(d) = NaiveDate::from_ymd_opt(y, m, start.day()) {
                        cands.push(d);
                    }
                }
                cands.sort();
                for d in cands {
                    if !e.push(d) {
                        break 'outer;
                    }
                }
                y += rule.interval as i32;
            }
        }
        _ => {}
    }
    e.out
}

// ───────────────────────── 쓰기 ─────────────────────────

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

/// 75바이트를 넘는 줄은 접는다(UTF-8 글자 중간은 자르지 않는다).
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut len = 0usize;
    for c in line.chars() {
        let w = c.len_utf8();
        if len + w > 74 {
            out.push_str("\r\n ");
            len = 1;
        }
        out.push(c);
        len += w;
    }
    out
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn parse_hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// 일정의 시작·종료를 로컬 NaiveDateTime으로. 종일이면 (시작일 0시, 종료일 다음 날 0시).
pub fn event_span(ev: &Event) -> Option<(NaiveDateTime, NaiveDateTime)> {
    let sd = parse_ymd(&ev.start_date)?;
    let ed = parse_ymd(&ev.end_date).unwrap_or(sd).max(sd);
    if ev.all_day || ev.start_time.is_none() {
        return Some((sd.and_time(NaiveTime::MIN), (ed + Duration::days(1)).and_time(NaiveTime::MIN)));
    }
    let st = ev.start_time.as_deref().and_then(parse_hm)?;
    let start = sd.and_time(st);
    let end = match ev.end_time.as_deref().and_then(parse_hm) {
        Some(et) => ed.and_time(et),
        None => start + Duration::hours(1),
    };
    Some((start, end.max(start)))
}

fn to_utc_string(ndt: NaiveDateTime) -> String {
    let utc = Local
        .from_local_datetime(&ndt)
        .earliest()
        .map(|l| l.with_timezone(&Utc).naive_utc())
        .unwrap_or(ndt);
    utc.format("%Y%m%dT%H%M%SZ").to_string()
}

fn vevent_lines(ev: &Event, uid: &str) -> Vec<String> {
    let mut l: Vec<String> = Vec::new();
    l.push("BEGIN:VEVENT".into());
    l.push(format!("UID:{}", uid));
    l.push(format!("DTSTAMP:{}", Utc::now().format("%Y%m%dT%H%M%SZ")));
    l.push(format!("SUMMARY:{}", escape(&ev.title)));
    if !ev.memo.trim().is_empty() {
        l.push(format!("DESCRIPTION:{}", escape(&ev.memo)));
    }
    if let Some((s, e)) = event_span(ev) {
        if ev.all_day || ev.start_time.is_none() {
            l.push(format!("DTSTART;VALUE=DATE:{}", s.format("%Y%m%d")));
            l.push(format!("DTEND;VALUE=DATE:{}", e.format("%Y%m%d")));
        } else {
            l.push(format!("DTSTART:{}", to_utc_string(s)));
            l.push(format!("DTEND:{}", to_utc_string(e)));
        }
    }
    let all_day = ev.all_day || ev.start_time.is_none();
    for m in &ev.reminders {
        let before = if all_day { *m + ALL_DAY_OFFSET_MIN } else { *m };
        l.push("BEGIN:VALARM".into());
        l.push("ACTION:DISPLAY".into());
        l.push("DESCRIPTION:Reminder".into());
        l.push(if before == 0 { "TRIGGER:PT0S".into() } else { format!("TRIGGER:-PT{}M", before) });
        l.push("END:VALARM".into());
    }
    l.push("END:VEVENT".into());
    l
}

fn wrap(body: Vec<String>) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//WidgetCalendar//KO".to_string(),
    ];
    lines.extend(body);
    lines.push("END:VCALENDAR".into());
    let mut out = String::new();
    for l in lines {
        out.push_str(&fold(&l));
        out.push_str("\r\n");
    }
    out
}

/// CalDAV 서버에 올릴 .ics 한 건
pub fn build_resource(ev: &Event, uid: &str) -> String {
    wrap(vevent_lines(ev, uid))
}

/// 백업용: 일정 전체를 .ics 하나로
pub fn export_calendar(events: &[Event]) -> String {
    let mut body = Vec::new();
    for ev in events.iter().filter(|e| !e.deleted) {
        body.extend(vevent_lines(ev, &format!("{}@widgetcalendar", ev.id)));
    }
    wrap(body)
}

// ───────────────────────── 테스트 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    fn titles(r: &ParseResult) -> Vec<String> {
        r.items.iter().map(|p| format!("{}~{}", p.event.start_date, p.event.end_date)).collect()
    }

    #[test]
    fn simple_and_allday() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:회의\\, 주간\r\nDESCRIPTION:첫째줄\\n둘째줄\r\nDTSTART:20261028T140000\r\nDTEND:20261028T153000\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nEND:VALARM\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:출장\r\nDTSTART;VALUE=DATE:20261029\r\nDTEND;VALUE=DATE:20261101\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let r = parse(ics, d("2026-10-01"), d("2026-12-31"));
        assert_eq!(r.vevent_count, 2);
        let a = &r.items[0].event;
        assert_eq!(a.title, "회의, 주간");
        assert_eq!(a.memo, "첫째줄\n둘째줄");
        assert_eq!((a.start_time.as_deref(), a.end_time.as_deref()), (Some("14:00"), Some("15:30")));
        assert_eq!(a.reminders, vec![10]);
        assert!(!a.all_day);
        let b = &r.items[1].event;
        assert!(b.all_day);
        assert_eq!((b.start_date.as_str(), b.end_date.as_str()), ("2026-10-29", "2026-10-31"));
    }

    #[test]
    fn weekly_byday_count_exdate_override() {
        // 매주 월·수, 총 6회, 10/7(수)은 제외, 10/12(월)은 다른 날로 이동
        let ics = "BEGIN:VEVENT\r\nUID:r\r\nSUMMARY:스터디\r\nDTSTART:20261005T190000\r\nDTEND:20261005T200000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=6\r\nEXDATE:20261007T190000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:r\r\nSUMMARY:스터디(변경)\r\nRECURRENCE-ID:20261012T190000\r\nDTSTART:20261013T100000\r\nDTEND:20261013T110000\r\nEND:VEVENT\r\n";
        let r = parse(ics, d("2026-10-01"), d("2026-12-31"));
        // 생성: 10/5, 10/7(제외), 10/12(예외로 대체), 10/14, 10/19, 10/21 → 10/5, 10/14, 10/19, 10/21 + 예외 10/13
        let mut got = titles(&r);
        got.sort();
        assert_eq!(
            got,
            vec!["2026-10-05~2026-10-05", "2026-10-13~2026-10-13", "2026-10-14~2026-10-14", "2026-10-19~2026-10-19", "2026-10-21~2026-10-21"]
        );
        assert!(r.items.iter().all(|p| p.recurring));
    }

    #[test]
    fn monthly_yearly_daily_until() {
        let w = (d("2026-01-01"), d("2028-12-31"));
        let mk = |rule: &str, start: &str| {
            format!("BEGIN:VEVENT\r\nUID:x\r\nSUMMARY:t\r\nDTSTART;VALUE=DATE:{}\r\nRRULE:{}\r\nEND:VEVENT\r\n", start, rule)
        };
        // 매월 31일: 31일이 없는 달은 건너뜀
        let r = parse(&mk("FREQ=MONTHLY;COUNT=4", "20260131"), w.0, w.1);
        assert_eq!(titles(&r).iter().map(|s| &s[..10]).collect::<Vec<_>>(), vec!["2026-01-31", "2026-03-31", "2026-05-31", "2026-07-31"]);
        // 매월 둘째 화요일
        let r = parse(&mk("FREQ=MONTHLY;BYDAY=2TU;COUNT=3", "20261013"), w.0, w.1);
        assert_eq!(titles(&r).iter().map(|s| &s[..10]).collect::<Vec<_>>(), vec!["2026-10-13", "2026-11-10", "2026-12-08"]);
        // 매월 마지막 금요일
        let r = parse(&mk("FREQ=MONTHLY;BYDAY=-1FR;COUNT=2", "20261030"), w.0, w.1);
        assert_eq!(titles(&r).iter().map(|s| &s[..10]).collect::<Vec<_>>(), vec!["2026-10-30", "2026-11-27"]);
        // 매년(윤일은 윤년에만)
        let r = parse(&mk("FREQ=YEARLY;COUNT=3", "20240229"), d("2024-01-01"), d("2040-01-01"));
        assert_eq!(titles(&r).iter().map(|s| &s[..10]).collect::<Vec<_>>(), vec!["2024-02-29", "2028-02-29", "2032-02-29"]);
        // 이틀마다, UNTIL까지
        let r = parse(&mk("FREQ=DAILY;INTERVAL=2;UNTIL=20260109", "20260101"), w.0, w.1);
        assert_eq!(r.items.len(), 5);
        // 구간 밖에서 시작한 반복도 구간 안 항목만
        let r = parse(&mk("FREQ=WEEKLY", "20150105"), d("2026-10-01"), d("2026-10-31"));
        assert_eq!(r.items.len(), 4); // 2026년 10월의 월요일: 5, 12, 19, 26
    }

    #[test]
    fn utc_and_tzid_convert_to_local() {
        let ics = "BEGIN:VEVENT\r\nUID:u\r\nSUMMARY:t\r\nDTSTART:20261028T050000Z\r\nDTEND:20261028T060000Z\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:v\r\nSUMMARY:t2\r\nDTSTART;TZID=Asia/Seoul:20261028T140000\r\nDTEND;TZID=Asia/Seoul:20261028T150000\r\nEND:VEVENT\r\n";
        let r = parse(ics, d("2026-10-01"), d("2026-12-31"));
        let expect_utc = Utc.with_ymd_and_hms(2026, 10, 28, 5, 0, 0).unwrap().with_timezone(&Local);
        assert_eq!(r.items[0].event.start_time.as_deref(), Some(expect_utc.format("%H:%M").to_string().as_str()));
        let seoul = chrono_tz::Asia::Seoul.with_ymd_and_hms(2026, 10, 28, 14, 0, 0).unwrap().with_timezone(&Local);
        assert_eq!(r.items[1].event.start_time.as_deref(), Some(seoul.format("%H:%M").to_string().as_str()));
    }

    #[test]
    fn allday_reminder_offset_and_roundtrip() {
        // 종일 일정: 1일 전 09:00 알림 = 자정 기준 -PT2340M(1440+900)
        let ev = Event {
            id: "1".into(),
            title: "생일; 파티, 케이크".into(),
            start_date: "2026-11-02".into(),
            end_date: "2026-11-03".into(),
            all_day: true,
            memo: "줄1\n줄2".into(),
            reminders: vec![0, 1440],
            ..Default::default()
        };
        let text = build_resource(&ev, "u1");
        assert!(text.contains("DTSTART;VALUE=DATE:20261102"));
        assert!(text.contains("DTEND;VALUE=DATE:20261104"));
        assert!(text.contains("TRIGGER:-PT540M"));
        assert!(text.contains("TRIGGER:-PT1980M"));
        let r = parse(&text, d("2026-10-01"), d("2026-12-31"));
        let e = &r.items[0].event;
        assert_eq!(e.title, ev.title);
        assert_eq!(e.memo, ev.memo);
        assert_eq!((e.start_date.as_str(), e.end_date.as_str()), ("2026-11-02", "2026-11-03"));
        assert_eq!(e.reminders, vec![0, 1440]);
    }

    #[test]
    fn timed_roundtrip_and_fold() {
        let ev = Event {
            id: "2".into(),
            title: "아주 긴 제목 ".repeat(20),
            start_date: "2026-11-02".into(),
            end_date: "2026-11-02".into(),
            start_time: Some("14:00".into()),
            end_time: Some("15:00".into()),
            reminders: vec![10],
            ..Default::default()
        };
        let text = build_resource(&ev, "u2");
        for line in text.split("\r\n") {
            assert!(line.len() <= 75, "줄이 너무 김: {}", line.len());
        }
        let r = parse(&text, d("2026-10-01"), d("2026-12-31"));
        let e = &r.items[0].event;
        assert_eq!(e.title, ev.title.trim());
        assert_eq!(e.start_time.as_deref(), Some("14:00"));
        assert_eq!(e.end_time.as_deref(), Some("15:00"));
        assert_eq!(e.reminders, vec![10]);
    }

    #[test]
    fn cancelled_and_window_filter() {
        let ics = "BEGIN:VEVENT\r\nUID:c\r\nSUMMARY:취소됨\r\nSTATUS:CANCELLED\r\nDTSTART:20261028T140000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:o\r\nSUMMARY:옛날\r\nDTSTART:20200101T100000\r\nEND:VEVENT\r\n";
        let r = parse(ics, d("2026-10-01"), d("2026-12-31"));
        assert_eq!(r.items.len(), 0);
        assert_eq!(r.vevent_count, 2);
    }
}
