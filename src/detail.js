// 알림을 눌렀을 때 뜨는 날짜 창. 위젯 안의 날짜 팝업과 같은 화면(dayview.js)을 쓴다.
import { api, applyTheme, applyLook, onSettingsChanged, toKey } from './common.js';
import { setCustomHolidays } from './koreancal.js';
import { createDayView } from './dayview.js';

const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

(async () => {
  let settings = await api.getSettings();
  applyTheme(settings.theme);
  applyLook(settings);
  setCustomHolidays(await api.getCustomHolidays().catch(() => []));
  onSettingsChanged((e) => {
    settings = { ...settings, ...e.payload };
    applyTheme(settings.theme);
    applyLook(settings);
  });
  const view = createDayView(document.getElementById('detail'), {
    date: new URLSearchParams(location.search).get('date') || toKey(new Date()),
    settings,
    onClose: () => getCurrentWindow().close(),
  });
  // 위젯이나 알림에서 다른 날짜를 누르면 이 창이 그대로 날짜만 바꾼다.
  listen('detail-date', (e) => view.setDate(e.payload));
})();
