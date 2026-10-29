import {
  api, THEMES, applyTheme, applyLook, onEventsChanged, onSettingsChanged, onAccountsChanged, onSettingsPreview,
  updateSettings, toKey, addDays, eventsOn, isAllDayLike, timeLabel, hmToMin, minToHm, DOW, el, pad,
} from './common.js';
import { buildEditor } from './editor.js';
import { createDayView } from './dayview.js';
import { dayItems, solarToLunar, lunarText, lunarLong, setCustomHolidays } from './koreancal.js';

const $ = (sel) => document.querySelector(sel);
const root = document.documentElement;
const grid = $('#grid');

let events = [];
let accounts = [];
let settings = { opacity: 0.85, monthMode: 'month', weekStart: 0, view: 'month', theme: 'midnight', showHolidays: true, showSpecial: true, showLunar: true, showPanel: true, fontScale: 1, fontFamily: '', surface: 0.5 };
let anchor = new Date(); // 화면에 보여줄 기준 날짜
let tlScroll = null;     // 주·일 보기의 세로 스크롤 위치(다시 그려도 유지)
let moveMode = false;

let HP = 48; // 주·일 보기에서 1시간의 높이(px). 글자 크기에 맞춰 커진다.

// ───────────────────────── 날짜 계산 ─────────────────────────

const startOfWeek = (d) => addDays(d, -((d.getDay() - settings.weekStart + 7) % 7));

function monthRange() {
  if (settings.monthMode === 'rolling') {
    // 오늘이 있는 주를 가운데 두고 위 2주, 아래 2주
    return { start: addDays(startOfWeek(anchor), -14), rows: 5 };
  }
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1);
  const daysInMonth = new Date(anchor.getFullYear(), anchor.getMonth() + 1, 0).getDate();
  const offset = (first.getDay() - settings.weekStart + 7) % 7;
  return { start: startOfWeek(first), rows: Math.ceil((offset + daysInMonth) / 7) };
}

const weekDays = () => Array.from({ length: 7 }, (_, i) => addDays(startOfWeek(anchor), i));

function isoWeek(d) {
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  t.setUTCDate(t.getUTCDate() + 4 - (t.getUTCDay() || 7));
  const y0 = new Date(Date.UTC(t.getUTCFullYear(), 0, 1));
  return Math.ceil(((t - y0) / 86400000 + 1) / 7);
}

function titleText() {
  const y = anchor.getFullYear();
  if (settings.view === 'month') return `${y}년 ${anchor.getMonth() + 1}월`;
  if (settings.view === 'day') return `${y}년 ${anchor.getMonth() + 1}월 ${anchor.getDate()}일 (${DOW[anchor.getDay()]})`;
  const [a, b] = [weekDays()[0], weekDays()[6]];
  return a.getMonth() === b.getMonth()
    ? `${y}년 ${a.getMonth() + 1}월 ${a.getDate()}일 – ${b.getDate()}일`
    : `${a.getMonth() + 1}월 ${a.getDate()}일 – ${b.getMonth() + 1}월 ${b.getDate()}일`;
}

// ───────────────────────── 공통 조각 ─────────────────────────

/** 하루치 일정 칩 목록(.events). 넘치면 아래가 흐려지고 hover하면 내려간다. */
function eventsBox(list) {
  const box = el('div', 'events');
  for (const ev of list) {
    if (ev._label) {
      // 공휴일·기념일·절기 이름
      // 좁은 칸에서 잘리지 않게 "대체공휴일(개천절)" → "개천절 대체"로 줄여 보이고, 전체 이름은 마우스를 올리면 보인다.
      const shown = ev.title.replace(/^대체공휴일\((.+)\)$/, '$1 대체');
      const h = el('div', `hday ${ev.off ? 'off' : ev.kind === 'term' ? 'term' : 'sp'}`, shown);
      h.title = ev.title;
      box.append(h);
      continue;
    }
    const chip = el('div', 'chip');
    chip.style.setProperty('--c', ev.color || '#5b9dff');
    if (ev._cont) chip.classList.add('cont'); // 여러 날 일정의 둘째 날부터
    if (ev.readOnly) chip.classList.add('ro');
    if (!isAllDayLike(ev)) chip.append(el('span', 't', ev.startTime));
    chip.append(el('span', 'x', ev.title));
    chip.title = ev.title;
    box.append(chip);
  }
  return box;
}

const withCont = (list, key) => list.map((e) => ({ ...e, _cont: key !== e.startDate }));

/** 그 날짜에 표시할 공휴일·기념일·절기 (설정의 표시 여부 반영) */
function labelItems(date) {
  return dayItems(date.getFullYear(), date.getMonth() + 1, date.getDate())
    .filter((it) => (it.kind === 'holiday' ? settings.showHolidays : settings.showSpecial))
    .map((it) => ({ _label: true, title: it.name, off: it.off, kind: it.kind }));
}
const lunarOf = (date) => (settings.showLunar ? solarToLunar(date.getFullYear(), date.getMonth() + 1, date.getDate()) : null);
function lunarEl(lun) {
  const e = el('span', `lunar${lun.day === 1 ? ' first' : ''}${lun.leap ? ' leap' : ''}`, lunarText(lun));
  e.title = lunarLong(lun);
  return e;
}
const wkOn = () => root.dataset.wk === 'on';

// ───────────────────────── 월 보기 ─────────────────────────

function renderMonth() {
  const { start, rows } = monthRange();
  const todayKey = toKey(new Date());
  grid.style.setProperty('--rows', rows);

  for (let i = 0; i < rows * 7; i++) {
    const date = addDays(start, i);
    if (wkOn() && i % 7 === 0) {
      // 한 행의 월요일이 속한 주의 번호(ISO)
      grid.append(el('div', 'wk', String(isoWeek(addDays(date, settings.weekStart === 0 ? 1 : 0)))));
    }
    const key = toKey(date);
    const cell = el('div', 'cell');
    cell.dataset.key = key;
    if (date.getDay() === 0) cell.classList.add('sun');
    if (date.getDay() === 6) cell.classList.add('sat');
    if (key === todayKey) cell.classList.add('today');
    if (settings.monthMode === 'month' && date.getMonth() !== anchor.getMonth()) cell.classList.add('dim');
    const labels = labelItems(date);
    if (labels.some((l) => l.off)) cell.classList.add('hol');
    // 양력 숫자 바로 옆에 음력을 작게 붙인다.
    const nrow = el('div', 'nrow');
    nrow.append(el('div', 'num', String(date.getDate())));
    const lun = lunarOf(date);
    if (lun) nrow.append(lunarEl(lun));
    cell.append(nrow);
    cell.append(eventsBox([...labels, ...withCont(eventsOn(events, key), key)]));
    grid.append(cell);
  }
}

// ───────────────────────── 주·일 보기 ─────────────────────────

/** 겹치는 일정을 나란히 배치한다. */
function layoutSegments(segs) {
  const out = [];
  let cluster = [];
  let clusterEnd = -1;
  const flush = () => {
    const cols = [];
    for (const s of cluster) {
      let c = cols.findIndex((end) => end <= s.s);
      if (c < 0) {
        c = cols.length;
        cols.push(s.e);
      } else cols[c] = s.e;
      s.col = c;
    }
    cluster.forEach((s) => (s.cols = cols.length));
    out.push(...cluster);
    cluster = [];
  };
  for (const s of segs) {
    if (cluster.length && s.s >= clusterEnd) {
      flush();
      clusterEnd = -1;
    }
    cluster.push(s);
    clusterEnd = Math.max(clusterEnd, s.e);
  }
  if (cluster.length) flush();
  return out;
}

/** 그날 시간표에 그릴 구간(분 단위) 목록 */
function timedSegments(key, list) {
  const segs = [];
  for (const ev of list) {
    if (isAllDayLike(ev)) continue;
    const s = ev.startDate === key ? hmToMin(ev.startTime) : 0;
    let e = ev.endDate === key ? (ev.endTime ? hmToMin(ev.endTime) : s + 60) : 1440;
    e = Math.min(1440, Math.max(e, s + 25)); // 너무 짧아도 읽을 수 있게 최소 높이
    segs.push({ ev, s: Math.min(s, 1440 - 25), e });
  }
  segs.sort((a, b) => a.s - b.s || b.e - a.e);
  return layoutSegments(segs);
}

function renderTimeline(days) {
  const todayKey = toKey(new Date());
  const n = days.length;
  grid.style.setProperty('--n', n);
  grid.style.setProperty('--hp', `${HP}px`);

  // 머리줄: 요일·날짜 + 종일 일정
  const head = el('div', 'tl-head');
  head.append(el('div', 'tl-gutter'));
  const hdays = el('div', 'tl-days');
  for (const d of days) {
    const key = toKey(d);
    const dh = el('div', 'dh');
    dh.dataset.key = key;
    if (d.getDay() === 0) dh.classList.add('sun');
    if (d.getDay() === 6) dh.classList.add('sat');
    if (key === todayKey) dh.classList.add('today');
    const top = el('div', 'dh-top');
    top.append(el('span', 'dw', DOW[d.getDay()]), el('span', 'dn', String(d.getDate())));
    const lun = lunarOf(d);
    if (lun) top.append(lunarEl(lun));
    const labels = labelItems(d);
    if (labels.some((l) => l.off)) dh.classList.add('hol');
    const allDay = eventsOn(events, key).filter(isAllDayLike);
    dh.append(top, eventsBox([...labels, ...withCont(allDay, key)]));
    hdays.append(dh);
  }
  head.append(hdays);

  // 본문: 시간 눈금 + 날짜별 열
  const scroll = el('div', 'tl-scroll');
  const body = el('div', 'tl-body');
  const gutter = el('div', 'tl-gutter hours');
  for (let h = 1; h < 24; h++) {
    const lab = el('span', null, `${pad(h)}:00`);
    lab.style.top = `${h * HP}px`;
    gutter.append(lab);
  }
  const cols = el('div', 'tl-cols');
  for (const d of days) {
    const key = toKey(d);
    const col = el('div', 'col');
    col.dataset.key = key;
    if (key === todayKey) col.classList.add('today');
    for (const seg of timedSegments(key, eventsOn(events, key))) {
      const { ev } = seg;
      const b = el('div', 'tev');
      b.style.setProperty('--c', ev.color || '#5b9dff');
      b.style.top = `${(seg.s / 60) * HP}px`;
      b.style.height = `${((seg.e - seg.s) / 60) * HP - 2}px`;
      b.style.left = `calc(${(seg.col / seg.cols) * 100}% + 2px)`;
      b.style.width = `calc(${100 / seg.cols}% - 4px)`;
      if (ev.readOnly) b.classList.add('ro');
      b.append(el('div', 'tt', ev.title));
      if (((seg.e - seg.s) / 60) * HP >= 38) {
        b.append(el('div', 'tm', ev.endTime ? `${ev.startTime}–${ev.endTime}` : ev.startTime));
      }
      b.title = ev.title;
      col.append(b);
    }
    if (key === todayKey) col.append(el('div', 'now'));
    cols.append(col);
  }
  body.append(gutter, cols);
  scroll.append(body);
  grid.append(head, scroll);
  updateNowLine();

  const hasToday = days.some((d) => toKey(d) === todayKey);
  const nowMin = new Date().getHours() * 60 + new Date().getMinutes();
  scroll.scrollTop = tlScroll ?? (hasToday ? Math.max(0, ((nowMin - 90) / 60) * HP) : 8 * HP);
  scroll.addEventListener('scroll', () => (tlScroll = scroll.scrollTop));
}

function updateNowLine() {
  const t = new Date();
  const y = ((t.getHours() * 60 + t.getMinutes()) / 60) * HP;
  grid.querySelectorAll('.now').forEach((n) => (n.style.top = `${y}px`));
}

// 빈 시간대를 클릭하면 그 시각으로 새 일정, 일정 위를 클릭하면 상세 창
grid.addEventListener('click', (e) => {
  const col = e.target.closest('.col');
  if (!col || dragging) return;
  const key = col.dataset.key;
  if (e.target.closest('.tev')) {
    openDayOverlay(key, e.target.closest('.tev'));
    return;
  }
  const y = e.clientY - col.getBoundingClientRect().top;
  const start = Math.max(0, Math.min(23 * 60 + 30, Math.floor((y / HP) * 2) * 30));
  openQuickAdd(key, key, { allDay: false, startTime: minToHm(start), endTime: minToHm(Math.min(start + 60, 23 * 60 + 59)) });
});

// ───────────────────────── 보조 패널(테마에 따라 표시) ─────────────────────────

const sortKey = (e) => (isAllDayLike(e) ? '' : e.startTime);

/** 오늘 이후의 일정(진행 중인 여러 날 일정은 오늘로 취급) */
function upcoming(limit) {
  const today = toKey(new Date());
  return events
    .filter((e) => e.endDate >= today)
    .map((e) => ({ e, day: e.startDate > today ? e.startDate : today }))
    .sort((a, b) => a.day.localeCompare(b.day) || sortKey(a.e).localeCompare(sortKey(b.e)) || a.e.title.localeCompare(b.e.title, 'ko'))
    .slice(0, limit);
}

function pitem(ev) {
  const it = el('div', 'pitem');
  it.style.setProperty('--c', ev.color || '#5b9dff');
  it.append(el('div', 'pt', timeLabel(ev)), el('div', 'pn', ev.title));
  return it;
}

const dayLabel = (key) => {
  const [y, m, d] = key.split('-').map(Number);
  return `${m}월 ${d}일 (${DOW[new Date(y, m - 1, d).getDay()]})`;
};

function renderPanels() {
  // 테마가 가진 패널 종류(root.dataset.panel)를, 사용자가 켜 둔 경우에만 보여 준다.
  const mode = settings.showPanel === false ? 'none' : root.dataset.panel || 'none';
  const hero = $('#hero'), agenda = $('#agenda'), strip = $('#strip');
  hero.hidden = mode !== 'hero';
  agenda.hidden = mode !== 'agenda';
  strip.hidden = mode !== 'strip';
  const today = toKey(new Date());

  if (mode === 'hero') {
    const now = new Date();
    const list = el('div', 'plist');
    const todays = eventsOn(events, today);
    if (todays.length === 0) list.append(el('div', 'pempty', '오늘은 일정이 없어요.'));
    todays.forEach((ev) => list.append(pitem(ev)));
    const lunNow = lunarOf(now);
    const sub = [lunNow ? lunarLong(lunNow) : null, ...labelItems(now).map((l) => l.title)].filter(Boolean).join(' · ');
    hero.replaceChildren(
      el('div', 'hero-day', String(now.getDate())),
      el('div', 'hero-dow', `${DOW[now.getDay()]}요일`),
      el('div', 'hero-month', `${now.getFullYear()}년 ${now.getMonth() + 1}월`),
      ...(sub ? [el('div', 'hero-sub', sub)] : []),
      el('h3', null, '오늘의 일정'),
      list,
    );
  } else if (mode === 'agenda') {
    const list = el('div', 'plist');
    const items = upcoming(14);
    if (items.length === 0) list.append(el('div', 'pempty', '다가오는 일정이 없어요.'));
    let last = '';
    for (const { e, day } of items) {
      if (day !== last) {
        list.append(el('div', 'pgroup', day === today ? `${dayLabel(day)} · 오늘` : dayLabel(day)));
        last = day;
      }
      list.append(pitem(e));
    }
    agenda.replaceChildren(el('h3', null, '다가오는 일정'), list);
  } else if (mode === 'strip') {
    const items = upcoming(8);
    strip.replaceChildren();
    if (items.length === 0) strip.append(el('div', 'pempty', '다가오는 일정이 없어요.'));
    for (const { e, day } of items) {
      const c = el('div', 'scard');
      c.style.setProperty('--c', e.color || '#5b9dff');
      const [, m, d] = day.split('-').map(Number);
      c.append(el('div', 'sd', `${m}/${d} · ${timeLabel(e)}`), el('div', 'sn', e.title));
      strip.append(c);
    }
  }
}

// ───────────────────────── 렌더 ─────────────────────────

function renderDow() {
  const dow = $('#dow');
  dow.replaceChildren();
  if (wkOn()) dow.append(el('div', 'wk-sp'));
  for (let i = 0; i < 7; i++) {
    const d = (settings.weekStart + i) % 7;
    dow.append(el('div', d === 0 ? 'sun' : d === 6 ? 'sat' : '', DOW[d]));
  }
}

function render() {
  const view = settings.view;
  grid.className = `grid ${view === 'month' ? 'month' : 'tl'}`;
  grid.replaceChildren();
  $('#dow').hidden = view !== 'month';
  document.querySelectorAll('#seg button').forEach((b) => b.classList.toggle('on', b.dataset.view === view));
  $('#title').textContent = titleText();
  $('#seg').style.setProperty('--i', ['day', 'week', 'month'].indexOf(view));

  if (view === 'month') renderMonth();
  else renderTimeline(view === 'week' ? weekDays() : [new Date(anchor.getFullYear(), anchor.getMonth(), anchor.getDate())]);
  requestAnimationFrame(refreshOverflow);
}

/** 테마·설정이 바뀌었을 때 전체를 다시 그린다. */
function applyAll() {
  const t = applyTheme(settings.theme);
  root.style.setProperty('--alpha', settings.opacity);
  applyLook(settings, { scale: true, surface: true }); // 글꼴·글자 크기·바탕 진하기·후광
  HP = Math.round(48 * Math.min(1.6, Math.max(0.8, settings.fontScale || 1)));
  $('#mi-theme-state').textContent = t.name;
  const mp = $('#mi-panel');
  const hasPanel = (root.dataset.panel || 'none') !== 'none';
  mp.setAttribute('aria-checked', String(settings.showPanel !== false));
  mp.classList.toggle('dim', !hasPanel);
  mp.title = hasPanel ? '' : '이 테마에는 오늘 일정 패널이 없습니다';
  renderDow();
  render();
  renderPanels();
}

// ───────────────────── 넘침 처리: 흐림 + hover 자동 스크롤 ─────────────────────

const FADE_ZONE = 30;      // 흐려진 영역(아래쪽 px)에 마우스가 들어오면 내려가기 시작
const SCROLL_DOWN = 0.035; // px/ms. 초당 약 35px
const SCROLL_UP = 0.15;    // 마우스가 벗어나면 맨 위로 되돌아오는 속도

function updateMore(box) {
  box.classList.toggle('more', box.scrollTop + box.clientHeight < box.scrollHeight - 1);
  box.classList.toggle('scrolled', box.scrollTop > 0);
}

function refreshOverflow() {
  document.querySelectorAll('.events').forEach(updateMore);
}

function stopScroll(box) {
  cancelAnimationFrame(box._raf);
  box._dir = 0;
}

function startScroll(box, dir) {
  if (box._dir === dir) return;
  stopScroll(box);
  box._dir = dir;
  // scrollTop은 소수점이 잘릴 수 있어 위치를 직접 들고 있는다.
  let pos = box.scrollTop;
  let last = performance.now();
  const step = (now) => {
    const dt = now - last;
    last = now;
    pos += dir * (dir > 0 ? SCROLL_DOWN : SCROLL_UP) * dt;
    const max = box.scrollHeight - box.clientHeight;
    pos = Math.max(0, Math.min(max, pos));
    box.scrollTop = pos;
    updateMore(box);
    if ((dir > 0 && pos >= max) || (dir < 0 && pos <= 0)) {
      box._dir = 0;
      return;
    }
    box._raf = requestAnimationFrame(step);
  };
  box._raf = requestAnimationFrame(step);
}

grid.addEventListener('mousemove', (e) => {
  if (dragging) return;
  const box = e.target.closest('.events');
  if (!box) return;
  const rect = box.getBoundingClientRect();
  const inFadeZone = box.classList.contains('more') && e.clientY > rect.bottom - FADE_ZONE;
  if (inFadeZone) startScroll(box, 1);
  else if (box._dir > 0) stopScroll(box); // 흐린 곳을 벗어나면 현재 위치에서 멈춤
});

grid.addEventListener('mouseout', (e) => {
  const box = e.target.closest('.events');
  if (!box || box.contains(e.relatedTarget)) return;
  if (box.scrollTop > 0) startScroll(box, -1); // 칸을 벗어나면 맨 위로 복귀
  else stopScroll(box);
});

new ResizeObserver(() => refreshOverflow()).observe(grid);

// ───────────────────── 날짜 클릭 / 드래그 선택 ─────────────────────

let dragging = false;
let dragStart = null;
let dragEnd = null;

const SELECTABLE = '.cell, .dh'; // 월 보기의 날짜 칸, 주·일 보기의 날짜 머리줄

function paintSelection() {
  const [a, b] = dragStart <= dragEnd ? [dragStart, dragEnd] : [dragEnd, dragStart];
  grid.querySelectorAll(SELECTABLE).forEach((c) => {
    c.classList.toggle('sel', c.dataset.key >= a && c.dataset.key <= b);
  });
}

grid.addEventListener('mousedown', (e) => {
  if (e.button !== 0) return;
  const cell = e.target.closest(SELECTABLE);
  if (!cell) return;
  e.preventDefault(); // 드래그 중 글자가 선택되지 않게
  dragging = true;
  dragStart = dragEnd = cell.dataset.key;
  paintSelection();
});

grid.addEventListener('mouseover', (e) => {
  if (!dragging) return;
  const cell = e.target.closest(SELECTABLE);
  if (!cell || cell.dataset.key === dragEnd) return;
  dragEnd = cell.dataset.key;
  paintSelection();
});

window.addEventListener('mouseup', () => {
  if (!dragging) return;
  const [a, b] = dragStart <= dragEnd ? [dragStart, dragEnd] : [dragEnd, dragStart];
  grid.querySelectorAll('.sel').forEach((c) => c.classList.remove('sel'));
  // click 이벤트(시간표 열)와 겹치지 않도록 다음 틱에 해제
  setTimeout(() => (dragging = false), 0);
  if (a === b) openDayOverlay(a, grid.querySelector(`[data-key="${a}"]`)); // 한 칸 클릭 → 그 칸에서 펼쳐지는 날짜 팝업
  else openQuickAdd(a, b);          // 여러 칸 드래그 → 기간 일정 등록
});

// ───────────────────── 빠른 등록 · 테마 선택 창 ─────────────────────

const modal = $('#modal');

function closeModal() {
  modal.hidden = true;
  modal.replaceChildren();
}

// 여러 날을 끌어서 고르거나 시간표의 빈 곳을 눌렀을 때: 그 칸에서 바로 입력 폼이 펼쳐진다.
function openQuickAdd(startDate, endDate, extra = {}) {
  openDayOverlay(startDate, grid.querySelector(`[data-key="${startDate}"]`), { event: null, defaults: { startDate, endDate, ...extra } });
}

// ───────────────────── 날짜 팝업: 클릭한 칸에서 달력 앞으로 펼쳐진다 ─────────────────────

const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
let dayOv = null;
let ovSeq = 0;

/** 팝업 카드를 클릭한 칸의 위치·크기로 줄여 놓는 변형값 */
function cardFrom(card, rect) {
  const f = card.getBoundingClientRect();
  if (!rect || !f.width || !f.height) return 'scale(0.92)';
  const s = Math.max(0.1, Math.min(rect.width / f.width, rect.height / f.height));
  const dx = rect.left + rect.width / 2 - (f.left + f.width / 2);
  const dy = rect.top + rect.height / 2 - (f.top + f.height / 2);
  return `translate(${dx}px, ${dy}px) scale(${s})`;
}

function openDayOverlay(key, anchor, edit = null) {
  if (dayOv) closeDayOverlay(true);
  setMenu(false);
  setYm(false);
  const my = ++ovSeq;
  const ov = $('#dayov');
  const card = $('#dayview');
  const rect = anchor ? anchor.getBoundingClientRect() : null;
  card.replaceChildren();
  card.style.transition = 'none';
  card.style.transform = '';
  card.style.opacity = '';
  ov.hidden = false;
  ov.classList.remove('in');
  const view = createDayView(card, {
    date: key,
    edit,
    events,
    accounts,
    settings,
    onClose: () => closeDayOverlay(),
    onSaved: () => loadEvents(),
  });
  dayOv = { view, rect, my };
  if (reducedMotion()) {
    card.style.opacity = '1';
  } else {
    card.style.transform = cardFrom(card, rect);
    card.style.opacity = '0';
    card.getBoundingClientRect(); // 시작 상태를 확정한 뒤
    card.style.transition = 'transform 0.4s cubic-bezier(0.2, 0.9, 0.25, 1.06), opacity 0.22s ease-out';
    card.style.transform = 'none';
    card.style.opacity = '1';
  }
  ov.classList.add('in');
}

function closeDayOverlay(immediate = false) {
  if (!dayOv) return;
  const { view, rect } = dayOv;
  dayOv = null;
  const my = ++ovSeq;
  const ov = $('#dayov');
  const card = $('#dayview');
  const finish = () => {
    view.destroy();
    if (my !== ovSeq) return; // 그 사이 새 팝업이 열렸으면 건드리지 않는다
    ov.hidden = true;
    ov.classList.remove('in');
    card.replaceChildren();
    card.style.transition = 'none';
    card.style.transform = '';
    card.style.opacity = '';
  };
  if (immediate || reducedMotion()) {
    finish();
    return;
  }
  ov.classList.remove('in');
  card.style.transition = 'transform 0.28s cubic-bezier(0.4, 0, 0.8, 0.4), opacity 0.2s ease-in 0.06s';
  card.style.transform = cardFrom(card, rect);
  card.style.opacity = '0';
  setTimeout(finish, 300);
}

$('#dayov').addEventListener('mousedown', (e) => {
  if (e.target === e.currentTarget) closeDayOverlay(); // 바깥을 누르면 닫힌다
});

// ───────────────────── 연·월 바로 가기 ─────────────────────

const ym = $('#ympop');
const YM_MIN = 1900;
const YM_MAX = 2100;
let ymYear = new Date().getFullYear();

function renderYm() {
  const now = new Date();
  const g = $('#ym-grid');
  g.replaceChildren();
  for (let m = 0; m < 12; m++) {
    const b = el('button', 'ym-m', `${m + 1}월`);
    b.type = 'button';
    if (ymYear === anchor.getFullYear() && m === anchor.getMonth()) b.classList.add('ym-cur');
    if (ymYear === now.getFullYear() && m === now.getMonth()) b.classList.add('ym-now');
    b.addEventListener('click', () => {
      jumpTo(ymYear, m);
      setYm(false);
    });
    g.append(b);
  }
  $('#ym-year').value = String(ymYear);
}

function setYm(open) {
  if (open) {
    ymYear = anchor.getFullYear();
    const w = $('#widget').getBoundingClientRect();
    const t = $('#title').getBoundingClientRect();
    const width = ym.getBoundingClientRect().width || 240;
    ym.hidden = false;
    const left = Math.max(6, Math.min(t.left - w.left, w.width - (ym.getBoundingClientRect().width || width) - 6));
    ym.style.left = `${Math.round(left)}px`;
    ym.style.top = `${Math.round(t.bottom - w.top + 8)}px`;
    renderYm();
  } else {
    ym.hidden = true;
  }
}

/** 그 연·월로 이동(같은 날짜 번호를 유지하되 그 달의 마지막 날을 넘지 않게) */
function jumpTo(y, m) {
  const last = new Date(y, m + 1, 0).getDate();
  anchor = new Date(y, m, Math.min(anchor.getDate(), last));
  render();
}

const clampYear = (v) => Math.min(YM_MAX, Math.max(YM_MIN, Math.round(Number(v)) || ymYear));
$('#ym-prev').addEventListener('click', () => { ymYear = clampYear(ymYear - 1); renderYm(); });
$('#ym-next').addEventListener('click', () => { ymYear = clampYear(ymYear + 1); renderYm(); });
$('#ym-year').addEventListener('change', (e) => { ymYear = clampYear(e.target.value); renderYm(); });
$('#ym-year').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') {
    ymYear = clampYear(e.target.value);
    renderYm();
  }
});
$('#ym-today').addEventListener('click', () => {
  goToday();
  setYm(false);
});
document.addEventListener('mousedown', (e) => {
  if (!ym.hidden && !ym.contains(e.target) && !$('#title').contains(e.target)) setYm(false);
});
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && !ym.hidden) setYm(false);
});

const HDR_NAME = { top: '상단 막대', journal: '큰 월 제목', centered: '가운데 제목', minimal: '미니멀 선', bottom: '하단 막대', pill: '알약 막대', stack: '2단 헤더', rail: '세로 막대', compact: '컴팩트 막대' };
const PANEL_NAME = { hero: '오늘 패널', agenda: '다가오는 일정', strip: '카드 줄' };

const THEME_CATS = ['깔끔한', '포근한', '눈이 편안한', '특별한', '시원한'];

function openThemePicker() {
  const sheet = el('div', 'sheet wide');
  sheet.append(el('h2', null, '테마'));
  const cards = [];
  for (const cat of THEME_CATS) {
    const list = THEMES.filter((t) => t.cat === cat);
    if (!list.length) continue;
    sheet.append(el('h3', 'theme-cat', cat));
    const gridEl = el('div', 'theme-grid');
    for (const t of list) {
      const b = el('button', 'tcard');
      b.type = 'button';
      b.style.setProperty('--tp', t.colors.panel);
      b.style.setProperty('--ti', t.colors.ink);
      b.style.setProperty('--tm', t.colors.muted);
      b.style.setProperty('--ta', t.colors.accent);
      if (t.id === settings.theme) b.classList.add('cur');
      const dots = el('div', 'dots');
      for (const c of [t.colors.accent, t.colors.sun, t.colors.sat]) {
        const i = document.createElement('i');
        i.style.background = c;
        dots.append(i);
      }
      const layout = [HDR_NAME[t.layout.hdr], PANEL_NAME[t.layout.panel]].filter(Boolean).join(' · ');
      b.append(dots, el('b', null, t.name), el('small', null, t.mood), el('small', null, layout));
      b.addEventListener('click', () => {
        settings.theme = t.id;
        applyAll();
        updateSettings({ theme: t.id });
        cards.forEach((x) => x.classList.toggle('cur', x === b));
      });
      cards.push(b);
      gridEl.append(b);
    }
    sheet.append(gridEl);
  }
  const foot = el('div', 'foot-row');
  const done = el('button', 'btn primary', '닫기');
  done.addEventListener('click', closeModal);
  foot.append(done);
  sheet.append(foot);
  modal.replaceChildren(sheet);
  modal.hidden = false;
}

modal.addEventListener('mousedown', (e) => {
  if (e.target === modal) closeModal();
});

// ───────────────────── 이동 · 보기 전환 ─────────────────────

function shift(dir) {
  if (settings.view === 'day') anchor = addDays(anchor, dir);
  else if (settings.view === 'week') anchor = addDays(anchor, dir * 7);
  else if (settings.monthMode === 'rolling') anchor = addDays(anchor, dir * 7);
  else anchor = new Date(anchor.getFullYear(), anchor.getMonth() + dir, 1);
  render();
}

$('#prev').addEventListener('click', () => shift(-1));
$('#next').addEventListener('click', () => shift(1));
const goToday = () => {
  anchor = new Date();
  tlScroll = null;
  render();
};
$('#today').addEventListener('click', goToday);
$('#title').addEventListener('click', (e) => {
  e.stopPropagation();
  setYm(ym.hidden);
}); // 제목을 누르면 연·월 선택

$('#seg').addEventListener('click', (e) => {
  const b = e.target.closest('button[data-view]');
  if (!b || b.dataset.view === settings.view) return;
  settings.view = b.dataset.view;
  tlScroll = null;
  updateSettings({ view: settings.view });
  render();
});

// ───────────────────── 메뉴 · 이동 모드 ─────────────────────

const menu = $('#menu');
// 메뉴는 테마(헤더 높이)와 상관없이 항상 메뉴 아이콘 바로 아래, 아이콘의 오른쪽 끝에 맞춰 연다.
const setMenu = (open) => {
  if (open) {
    const w = $('#widget').getBoundingClientRect();
    const b = $('#btn-menu').getBoundingClientRect();
    menu.style.top = `${Math.round(b.bottom - w.top + 6)}px`;
    menu.style.right = `${Math.max(6, Math.round(w.right - b.right))}px`;
    menu.style.bottom = 'auto';
    menu.style.left = 'auto';
  }
  menu.hidden = !open;
};

$('#btn-menu').addEventListener('click', (e) => {
  e.stopPropagation();
  setMenu(menu.hidden);
});
document.addEventListener('mousedown', (e) => {
  if (!menu.hidden && !menu.contains(e.target) && !$('#btn-menu').contains(e.target)) setMenu(false);
});

async function setMove(on) {
  moveMode = on;
  $('#movelayer').hidden = !on;
  $('#mi-move').classList.toggle('on', on);
  $('#mi-move-state').textContent = on ? '이동 중' : '고정됨';
  await api.setMoveMode(on).catch(() => {});
}

$('#mi-move').addEventListener('click', () => {
  setMenu(false);
  setMove(!moveMode);
});
$('#move-done').addEventListener('click', () => setMove(false));
$('#mi-theme').addEventListener('click', () => {
  setMenu(false);
  openThemePicker();
});
// 슬라이딩 스위치: 누르면 바로 적용·저장하고, 움직이는 모습을 볼 수 있게 메뉴는 열어 둔다.
$('#mi-panel').addEventListener('click', () => {
  settings.showPanel = settings.showPanel === false;
  applyAll();
  updateSettings({ showPanel: settings.showPanel });
});
$('#mi-settings').addEventListener('click', () => {
  setMenu(false);
  api.openSettings();
});
$('#mi-hide').addEventListener('click', () => {
  setMenu(false);
  api.hideWidget();
});
$('#mi-quit').addEventListener('click', () => api.quitApp());

window.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (!modal.hidden) closeModal();
  else if (!menu.hidden) setMenu(false);
  else if (moveMode) setMove(false);
});

// ───────────────────── 시작 ─────────────────────

async function loadEvents() {
  events = await api.listEvents();
  render();
  renderPanels();
}

async function loadAccounts() {
  accounts = await api.listAccounts();
}

async function init() {
  settings = { ...settings, ...(await api.getSettings()) };
  await loadAccounts();
  events = await api.listEvents();
  setCustomHolidays(await api.getCustomHolidays().catch(() => []));
  applyAll();

  // 다른 창에서 바뀐 내용을 반영
  onEventsChanged(loadEvents);
  onAccountsChanged(loadAccounts);
  onSettingsChanged((e) => {
    const prevView = settings.view;
    settings = { ...settings, ...e.payload };
    if (prevView !== settings.view) tlScroll = null;
    applyAll();
    // 사용자가 holidays_custom.json 을 고쳤을 수 있으니 저장 때마다 다시 읽는다.
    api.getCustomHolidays().then((l) => { setCustomHolidays(l); applyAll(); }).catch(() => {});
  });
  // 설정 창에서 저장하기 전의 값을 미리 보여 준다(저장되지 않음. 취소하면 저장된 값으로 되돌아감).
  onSettingsPreview((e) => {
    settings = { ...settings, ...e.payload };
    applyAll();
  });

  // 1분마다: 현재 시각 선을 옮기고, 자정이 지나면 오늘 표시를 갱신한다.
  let day = toKey(new Date());
  setInterval(() => {
    const now = toKey(new Date());
    if (now !== day) {
      day = now;
      render();
      renderPanels();
    } else if (settings.view !== 'month') {
      updateNowLine();
    }
  }, 60_000);
}

init();
