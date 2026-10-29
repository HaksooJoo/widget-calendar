// 한국 달력 계산: 음력(윤달 포함), 공휴일(대체공휴일 포함), 기념일, 24절기, 삼복.
// 표 데이터는 kcal-data.js(자동 생성)에 있고, 지원 범위는 양력 1900-01-31 ~ 2050-12-31 이다.
import { LUNAR_START, LUNAR_DATA, TERMS_START, TERM_NAMES, TERM_DATES } from './kcal-data.js';

const DAY_MS = 86400000;
const pad = (n) => String(n).padStart(2, '0');
const keyOf = (y, m, d) => `${y}-${pad(m)}-${pad(d)}`;
/** 양력 날짜 → 1970-01-01 기준 일수 (시간대 영향이 없도록 UTC로 계산) */
const dayNum = (y, m, d) => Math.floor(Date.UTC(y, m - 1, d) / DAY_MS);
const fromDayNum = (n) => {
  const dt = new Date(n * DAY_MS);
  return { y: dt.getUTCFullYear(), m: dt.getUTCMonth() + 1, d: dt.getUTCDate() };
};
const weekday = (n) => (((n + 4) % 7) + 7) % 7; // 0=일요일 (1970-01-01은 목요일)

export const LUNAR_MIN_DAY = dayNum(1900, 1, 31);
export const LUNAR_MAX_DAY = dayNum(2050, 12, 31);

// ───────────────────────── 음력 ─────────────────────────

const newYearDay = (idx) => dayNum(LUNAR_START + idx, LUNAR_DATA[idx][0], LUNAR_DATA[idx][1]);

/** 그 해의 달 순서: 1월, 2월, … (윤달은 해당 월 바로 뒤) 와 각 달의 길이 */
function monthsOf(idx) {
  const [, , leapMonth, bits] = LUNAR_DATA[idx];
  const out = [];
  let i = 0;
  for (let m = 1; m <= 12; m++) {
    out.push({ month: m, leap: false, len: (bits >> i++) & 1 ? 30 : 29 });
    if (m === leapMonth) out.push({ month: m, leap: true, len: (bits >> i++) & 1 ? 30 : 29 });
  }
  return out;
}

/** 양력 → 음력. 범위 밖이면 null. 반환: { year, month, day, leap } */
export function solarToLunar(y, m, d) {
  const n = dayNum(y, m, d);
  if (n < LUNAR_MIN_DAY || n > LUNAR_MAX_DAY) return null;
  let idx = Math.min(Math.max(y - LUNAR_START, 0), LUNAR_DATA.length - 1);
  if (n < newYearDay(idx)) idx -= 1;
  let off = n - newYearDay(idx);
  for (const mo of monthsOf(idx)) {
    if (off < mo.len) return { year: LUNAR_START + idx, month: mo.month, day: off + 1, leap: mo.leap };
    off -= mo.len;
  }
  return null;
}

/** 음력 → 양력. 없는 날짜면 null. */
export function lunarToSolar(ly, lm, ld, leap = false) {
  const idx = ly - LUNAR_START;
  if (idx < 0 || idx >= LUNAR_DATA.length) return null;
  let off = 0;
  for (const mo of monthsOf(idx)) {
    if (mo.month === lm && mo.leap === leap) {
      if (ld < 1 || ld > mo.len) return null;
      return fromDayNum(newYearDay(idx) + off + ld - 1);
    }
    off += mo.len;
  }
  return null;
}

/** 음력 표시 문자열: "9.18", 윤달은 "윤6.17" */
export const lunarText = (l) => `${l.leap ? '윤' : ''}${l.month}.${l.day}`;
/** 음력 한글 표기: "음력 윤6월 17일" */
export const lunarLong = (l) => `음력 ${l.leap ? '윤' : ''}${l.month}월 ${l.day}일`;

function lastDayOfLunarMonth(ly, lm) {
  const idx = ly - LUNAR_START;
  const mo = monthsOf(idx).find((x) => x.month === lm && !x.leap);
  return mo ? mo.len : 29;
}

// ───────────────────────── 24절기 · 삼복 · 한식 ─────────────────────────

/** 그 해의 24절기: [{ name, month, day }] (소한부터 동지까지, 한국 표준시 기준) */
export function solarTerms(year) {
  const row = TERM_DATES[year - TERMS_START];
  if (!row) return [];
  return row.map((v, i) => ({ name: TERM_NAMES[i], month: Math.floor(v / 100), day: v % 100 }));
}

/** 그 날의 천간이 경(庚)일인지. (2000-01-01이 무오(戊午)일인 것을 기준으로 계산) */
const isGyeong = (n) => (n + 2440588 + 9) % 10 === 6;
const firstGyeongOnOrAfter = (n) => {
  while (!isGyeong(n)) n++;
  return n;
};

// ───────────────────────── 공휴일 · 기념일 ─────────────────────────

/** 선거일·임시공휴일처럼 규칙으로 정해지지 않는 공휴일. 새로 지정되는 임시공휴일은 holidays_custom.json 으로 추가한다. */
const FIXED_EXTRA = [
  ['2020-08-17', '임시공휴일'],
  ['2022-03-09', '제20대 대통령선거'],
  ['2022-06-01', '제8회 전국동시지방선거'],
  ['2023-10-02', '임시공휴일'],
  ['2024-04-10', '제22대 국회의원선거'],
  ['2024-10-01', '국군의 날(임시공휴일)'],
  ['2025-01-27', '임시공휴일'],
  ['2025-06-03', '제21대 대통령선거'],
  ['2026-06-03', '제9회 전국동시지방선거'],
  ['2028-04-12', '제23대 국회의원선거'],
  ['2030-06-05', '제10회 전국동시지방선거'],
];

let custom = []; // [{ date, name, off }]
const cache = new Map();

/** 사용자가 추가한 날짜(holidays_custom.json). off가 false면 쉬는 날이 아니라 기념일로 표시한다. */
export function setCustomHolidays(list) {
  custom = (Array.isArray(list) ? list : []).filter((x) => x && /^\d{4}-\d{2}-\d{2}$/.test(x.date) && x.name);
  cache.clear();
}

function buildYear(year) {
  /** key → [{ name, off, kind }] ; kind: holiday | special | term */
  const map = new Map();
  const add = (key, name, off, kind) => {
    if (!map.has(key)) map.set(key, []);
    map.get(key).push({ name, off, kind });
  };
  const addYMD = (m, d, name, off, kind) => add(keyOf(year, m, d), name, off, kind);
  const addDay = (n, name, off, kind) => {
    const { y, m, d } = fromDayNum(n);
    if (y === year) add(keyOf(y, m, d), name, off, kind);
  };

  // 1) 양력 고정 공휴일
  addYMD(1, 1, '신정', true, 'holiday');
  if (year >= 1949) addYMD(3, 1, '삼일절', true, 'holiday');
  if (year >= 1975) addYMD(5, 5, '어린이날', true, 'holiday');
  if (year >= 1956) addYMD(6, 6, '현충일', true, 'holiday');
  if (year >= 1949) addYMD(8, 15, '광복절', true, 'holiday');
  if (year >= 1949) addYMD(10, 3, '개천절', true, 'holiday');
  if (year >= 2013) addYMD(10, 9, '한글날', true, 'holiday');
  addYMD(12, 25, '성탄절', true, 'holiday');
  if (year <= 2007) addYMD(7, 17, '제헌절', true, 'holiday');
  if (year <= 2005) addYMD(4, 5, '식목일', true, 'holiday');

  // 2) 음력 공휴일: 설날(연휴 3일), 추석(연휴 3일), 부처님오신날
  const blocks = []; // 대체공휴일 계산용 [시작일, 이름]
  const seol = lunarToSolar(year, 1, 1);
  const chuseok = lunarToSolar(year, 8, 15);
  const buddha = lunarToSolar(year, 4, 8);
  for (const [s, name] of [[seol, '설날'], [chuseok, '추석']]) {
    if (!s) continue;
    const n = dayNum(s.y, s.m, s.d);
    const three = year >= 1990;
    if (three) addDay(n - 1, `${name} 연휴`, true, 'holiday');
    addDay(n, name, true, 'holiday');
    if (three) addDay(n + 1, `${name} 연휴`, true, 'holiday');
    blocks.push({ name, from: three ? n - 1 : n, to: three ? n + 1 : n });
  }
  if (buddha && year >= 1975) addDay(dayNum(buddha.y, buddha.m, buddha.d), '부처님오신날', true, 'holiday');

  // 3) 선거일·임시공휴일 + 사용자가 추가한 날짜
  for (const [date, name] of FIXED_EXTRA) if (date.startsWith(`${year}-`)) add(date, name, true, 'holiday');
  for (const c of custom) if (c.date.startsWith(`${year}-`)) add(c.date, c.name, c.off !== false, c.off !== false ? 'holiday' : 'special');

  // 4) 대체공휴일 (시행 시기: 설날·추석·어린이날 2014 / 삼일절·광복절·개천절·한글날 2021 / 부처님오신날·성탄절 2023)
  const isOff = (key) => (map.get(key) || []).some((x) => x.off);
  const nextWorkday = (n) => {
    n += 1;
    while (weekday(n) === 0 || weekday(n) === 6 || isOff(keyOf(fromDayNum(n).y, fromDayNum(n).m, fromDayNum(n).d))) n += 1;
    return n;
  };
  const subs = [];
  if (year >= 2014) {
    for (const b of blocks) {
      let hit = false;
      for (let n = b.from; n <= b.to; n++) {
        const { y, m, d } = fromDayNum(n);
        const others = (map.get(keyOf(y, m, d)) || []).filter((x) => x.off && !x.name.startsWith(b.name));
        if (weekday(n) === 0 || others.length) hit = true;
      }
      if (hit) subs.push({ after: b.to, label: `대체공휴일(${b.name})` });
    }
  }
  // 단일 공휴일: 토·일요일과 겹치거나(어린이날·부처님오신날은 다른 공휴일과 겹칠 때도) 하루에 한 번만 대체한다.
  const RULES = { 어린이날: [2014, true], 삼일절: [2021, false], 광복절: [2021, false], 개천절: [2021, false], 한글날: [2021, false], 부처님오신날: [2023, true], 성탄절: [2023, false] };
  for (const [key, items] of map) {
    const eligible = items.filter((x) => RULES[x.name] && year >= RULES[x.name][0]);
    if (!eligible.length) continue;
    const [y, m, d] = key.split('-').map(Number);
    const n = dayNum(y, m, d);
    const weekend = weekday(n) === 0 || weekday(n) === 6;
    const clash = items.filter((x) => x.off && !x.name.startsWith('대체')).length > 1 && eligible.some((x) => RULES[x.name][1]);
    if (weekend || clash) subs.push({ after: n, label: `대체공휴일(${eligible[0].name})` });
  }
  subs.sort((a, b) => a.after - b.after);
  for (const s of subs) {
    const n = nextWorkday(s.after);
    const { y, m, d } = fromDayNum(n);
    if (y === year) add(keyOf(y, m, d), s.label, true, 'holiday');
  }

  // 5) 기념일
  const SPECIAL = [[2, 14, '밸런타인데이'], [3, 14, '화이트데이'], [5, 1, '노동절'], [5, 8, '어버이날'], [5, 15, '스승의 날'], [5, 21, '부부의 날'], [6, 25, '6·25전쟁일'], [10, 1, '국군의 날'], [12, 24, '크리스마스 이브']];
  for (const [m, d, name] of SPECIAL) addYMD(m, d, name, false, 'special');
  if (year > 2005) addYMD(4, 5, '식목일', false, 'special');
  if (year > 2007) addYMD(7, 17, '제헌절', false, 'special');
  // 성년의 날: 5월 셋째 월요일
  {
    let n = dayNum(year, 5, 1);
    while (weekday(n) !== 1) n++;
    addDay(n + 14, '성년의 날', false, 'special');
  }
  // 음력 기념일
  for (const [lm, ld, name] of [[1, 15, '정월대보름'], [5, 5, '단오'], [7, 7, '칠석'], [7, 15, '백중'], [9, 9, '중양절']]) {
    const s = lunarToSolar(year, lm, ld);
    if (s) addDay(dayNum(s.y, s.m, s.d), name, false, 'special');
  }
  {
    // 섣달그믐: 음력 12월의 마지막 날. 음력 12월은 양력으로 이듬해 1~2월이므로 '전년도' 음력 12월을 본다.
    const s = lunarToSolar(year - 1, 12, lastDayOfLunarMonth(year - 1, 12));
    if (s) addDay(dayNum(s.y, s.m, s.d), '섣달그믐', false, 'special');
  }

  // 6) 24절기 · 삼복 · 한식
  const terms = solarTerms(year);
  for (const t of terms) addYMD(t.month, t.day, t.name, false, 'term');
  if (terms.length) {
    const t = (name) => terms.find((x) => x.name === name);
    const hajiN = dayNum(year, t('하지').month, t('하지').day);
    // 초복·중복은 하지 뒤 세 번째·네 번째 경일(경일은 10일마다 돌아오므로 첫 경일 +20, +30)
    const firstGyeong = firstGyeongOnOrAfter(hajiN);
    addDay(firstGyeong + 20, '초복', false, 'special');
    addDay(firstGyeong + 30, '중복', false, 'special');
    const ipchuN = dayNum(year, t('입추').month, t('입추').day);
    addDay(firstGyeongOnOrAfter(ipchuN), '말복', false, 'special');
    // 한식: 전년도 동지로부터 105일째
    const prev = solarTerms(year - 1).find((x) => x.name === '동지');
    if (prev) addDay(dayNum(year - 1, prev.month, prev.day) + 105, '한식', false, 'special');
  }

  // 같은 날 항목은 공휴일 → 기념일 → 절기 순으로 정렬
  const rank = { holiday: 0, special: 1, term: 2 };
  for (const arr of map.values()) arr.sort((a, b) => rank[a.kind] - rank[b.kind]);
  return map;
}

/** 해당 날짜의 공휴일·기념일·절기 목록. [{ name, off, kind }] */
export function dayItems(y, m, d) {
  if (!cache.has(y)) cache.set(y, buildYear(y));
  return cache.get(y).get(keyOf(y, m, d)) || [];
}

/** 쉬는 날(공휴일)인지 */
export const isHoliday = (y, m, d) => dayItems(y, m, d).some((x) => x.off);
