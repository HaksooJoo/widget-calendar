import { api, applyTheme, applyLook } from './common.js';

const { getCurrentWindow } = window.__TAURI__.window;
const q = new URLSearchParams(location.search);
const SHOW_MS = 10000; // 알림이 떠 있는 시간

document.getElementById('tt').textContent = q.get('title') || '일정 알림';
document.getElementById('tm').textContent = q.get('body') || '';
const color = q.get('color') || '#5b9dff';
document.getElementById('toast').style.setProperty('--c', color);

api.getSettings().then((s) => { applyTheme(s.theme); applyLook(s); }).catch(() => {});

const close = () => getCurrentWindow().close().catch(() => {});

// 마우스를 올려 두면 닫히는 시간이 멈춘다.
let remaining = SHOW_MS;
let last = performance.now();
let hovering = false;
const bar = document.getElementById('tprog');
const tick = (now) => {
  if (!hovering) remaining -= now - last;
  last = now;
  bar.style.transform = `scaleX(${Math.max(0, remaining / SHOW_MS)})`;
  if (remaining <= 0) return close();
  requestAnimationFrame(tick);
};
requestAnimationFrame(tick);
const toast = document.getElementById('toast');
toast.addEventListener('mouseenter', () => (hovering = true));
toast.addEventListener('mouseleave', () => (hovering = false));

document.getElementById('tclose').addEventListener('click', (e) => {
  e.stopPropagation();
  close();
});
// 알림을 누르면 그 날짜의 상세 창을 열고 알림은 닫는다.
toast.addEventListener('click', async () => {
  const date = q.get('date');
  if (date) await api.openDetail(date).catch(() => {});
  close();
});
