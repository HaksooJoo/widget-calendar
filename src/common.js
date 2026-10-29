// 위젯, 상세, 설정, 알림 창이 함께 쓰는 모듈.
import { THEMES } from './themes.js';

const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;

export { THEMES };

export const DEFAULT_FONT = "'Segoe UI Variable Text', 'Segoe UI', 'Malgun Gothic', system-ui, sans-serif";
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, Number.isFinite(v) ? v : lo));

/** 글꼴 이름 → CSS font-family 값. 이름이 비어 있으면 기본 글꼴. 없는 글꼴이면 뒤의 기본 글꼴로 대체된다. */
export function fontStack(name) {
  const n = String(name ?? '').replace(/["'\\;{}<>]/g, '').trim();
  return n ? `"${n}", ${DEFAULT_FONT}` : DEFAULT_FONT;
}

/** 이 PC에 설치된 글꼴인지(같은 글자를 기본 글꼴과 폭으로 비교) */
export function isFontInstalled(name) {
  const ctx = document.createElement('canvas').getContext('2d');
  const text = '가나다라마바사 abcdefghijklmnopqrstuvwxyz 0123456789';
  const width = (f) => {
    ctx.font = `72px ${f}`;
    return ctx.measureText(text).width;
  };
  return ['monospace', 'serif', 'sans-serif'].some((base) => {
    const w0 = width(base);
    return width(`"${name}", ${base}`) !== w0;
  });
}

/**
 * 글꼴·글자 크기·바탕 진하기·후광을 적용한다.
 * - scale: 글자 크기 배율을 쓸지(설정 창은 끈다)   - surface: 칸·일정 바탕 진하기를 쓸지(위젯만 켠다)
 */
export function applyLook(s, { scale = true, surface = false } = {}) {
  const root = document.documentElement;
  root.style.setProperty('--font', fontStack(s.fontFamily));
  // 사용자가 글꼴을 골랐으면 테마가 정한 제목용 글꼴(한지의 바탕체 등)보다 사용자의 선택을 우선한다.
  if (s.fontFamily) root.style.setProperty('--font-display', 'inherit');
  else root.style.removeProperty('--font-display');
  const fs = scale ? clamp(s.fontScale ?? 1, 0.8, 1.6) : 1;
  root.style.setProperty('--fs', fs);
  root.style.setProperty('--fs-toast', Math.min(fs, 1.2));

  // 칸·일정 바탕 진하기: 테마가 정한 바탕 투명도에 곱한다.
  root.style.removeProperty('--cell');
  root.style.removeProperty('--cell-hover');
  const k = surface ? clamp(s.surface ?? 0.5, 0, 1) : 1;
  root.style.setProperty('--sk', k);
  if (surface) {
    const cs = getComputedStyle(root);
    for (const name of ['--cell', '--cell-hover']) {
      const m = cs.getPropertyValue(name).match(/rgba?\(([^)]+)\)/);
      if (!m) continue;
      const [r, g, b, a = '1'] = m[1].split(',').map((x) => x.trim());
      root.style.setProperty(name, `rgba(${r}, ${g}, ${b}, ${(parseFloat(a) * k).toFixed(4)})`);
    }
  }

  // 후광: 위젯이 불투명할 때는 보이지 않고, 투명해질수록 글자 뒤에 배경색을 살짝 깔아 읽히게 한다.
  const opacity = clamp(s.opacity ?? 0.85, 0.2, 1);
  const pct = Math.round(clamp((0.93 - opacity) * 1.75, 0, 0.9) * 100);
  root.style.setProperty('--halo-pct', `${pct}%`);
}

/** 테마를 적용한다: 색은 data-theme, 레이아웃은 data-hdr/grid/chip/panel/num/wk 속성으로 */
export function applyTheme(id) {
  const t = THEMES.find((x) => x.id === id) ?? THEMES[0];
  const root = document.documentElement;
  root.dataset.theme = t.id;
  root.dataset.dark = t.dark ? '1' : '0';
  for (const [k, v] of Object.entries(t.layout)) root.dataset[k] = v;
  return t;
}

export const api = {
  // 일정
  listEvents: () => invoke('list_events'),
  saveEvent: (event) => invoke('save_event', { event }),
  deleteEvent: (id) => invoke('delete_event', { id }),
  // 설정
  getSettings: () => invoke('get_settings'),
  saveSettings: (settings) => invoke('save_settings', { settings }),
  getAutostart: () => invoke('get_autostart'),
  setAutostart: (enabled) => invoke('set_autostart', { enabled }),
  saveGoogleClient: (a) => invoke('save_google_client', a),
  getCustomHolidays: () => invoke('get_custom_holidays'),
  openDataFolder: () => invoke('open_data_folder'),
  testNotification: (style) => invoke('test_notification', { style }),
  // 창
  openDetail: (date) => invoke('open_detail', { date }),
  openSettings: () => invoke('open_settings'),
  hideWidget: () => invoke('hide_widget'),
  setMoveMode: (enabled) => invoke('set_move_mode', { enabled }),
  quitApp: () => invoke('quit_app'),
  // 동기화
  listAccounts: () => invoke('list_accounts'),
  googleLoginAvailable: () => invoke('google_login_available'),
  addGoogle: (a) => invoke('add_google_account', a),
  addCaldav: (a) => invoke('add_caldav_account', a),
  addIcs: (a) => invoke('add_ics_account', a),
  removeAccount: (id) => invoke('remove_account', { id }),
  syncNow: (id = null) => invoke('sync_now', { id }),
  // 백업
  backupNow: () => invoke('backup_now'),
  backupInfo: () => invoke('backup_info'),
  restoreBackup: () => invoke('restore_backup'),
  openBackupFolder: () => invoke('open_backup_folder'),
};

// 다른 창에서 바뀌면 Rust가 이 이벤트를 모든 창에 보낸다.
export const onEventsChanged = (fn) => listen('events-changed', fn);
export const onSettingsChanged = (fn) => listen('settings-changed', fn);
export const onAccountsChanged = (fn) => listen('accounts-changed', fn);
// 설정 창에서 아직 저장하지 않은 값을 위젯에 미리 보여 줄 때 쓴다(저장되지 않음).
export const onSettingsPreview = (fn) => listen('settings-preview', fn);
export const sendSettingsPreview = (patch) => emit('settings-preview', patch);

/** 다른 창이 바꾼 값을 덮어쓰지 않도록, 항상 최신 설정에 일부만 덮어 저장한다. */
export async function updateSettings(patch) {
  const cur = await api.getSettings();
  return api.saveSettings({ ...cur, ...patch });
}

// ── 날짜 유틸 (모두 로컬 시간 기준, 키는 "YYYY-MM-DD") ──
export const pad = (n) => String(n).padStart(2, '0');
export const toKey = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
export const fromKey = (k) => {
  const [y, m, d] = k.split('-').map(Number);
  return new Date(y, m - 1, d);
};
export const addDays = (d, n) => {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
};
export const DOW = ['일', '월', '화', '수', '목', '금', '토'];
export const hmToMin = (s) => {
  const [h, m] = s.split(':').map(Number);
  return h * 60 + m;
};
export const minToHm = (n) => `${pad(Math.floor(n / 60))}:${pad(n % 60)}`;

// ── 일정 ──
export const COLORS = ['#5b9dff', '#ff6b7a', '#3ec99a', '#f2a93b', '#a98bff', '#34c5d8', '#f272b0', '#8b94a8'];

/** 알림 선택지(분). 0은 정시. */
export const REMINDER_CHOICES = [0, 5, 10, 30, 60, 180, 1440, 10080];

export function reminderLabel(m) {
  if (m === 0) return '정시';
  if (m % 10080 === 0) return `${m / 10080}주 전`;
  if (m % 1440 === 0) return `${m / 1440}일 전`;
  if (m % 60 === 0) return `${m / 60}시간 전`;
  return `${m}분 전`;
}

/** 종일(또는 시간 없는) 일정인지 */
export const isAllDayLike = (e) => e.allDay || !e.startTime;

/** 해당 날짜에 걸쳐 있는 일정. 종일 → 시간 순 → 제목 순. */
export function eventsOn(events, key) {
  const sortKey = (e) => (isAllDayLike(e) ? '' : e.startTime);
  return events
    .filter((e) => e.startDate <= key && key <= e.endDate)
    .sort((a, b) => sortKey(a).localeCompare(sortKey(b)) || a.title.localeCompare(b.title, 'ko'));
}

export function timeLabel(ev) {
  if (isAllDayLike(ev)) return '종일';
  return ev.endTime ? `${ev.startTime}–${ev.endTime}` : ev.startTime;
}

/** 작은 DOM 헬퍼. 텍스트는 항상 textContent로 넣어 HTML 주입을 막는다. */
export function el(tag, cls, text) {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
}

export function fmtDateTime(ms) {
  if (!ms) return '아직 없음';
  const d = new Date(ms);
  return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
