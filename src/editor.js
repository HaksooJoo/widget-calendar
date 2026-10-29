import { COLORS, REMINDER_CHOICES, reminderLabel, toKey } from './common.js';

/**
 * 일정 입력 폼을 만들어 돌려준다.
 * event가 있으면 수정, 없으면 새 일정(defaults로 채움).
 * calendars: 새 일정을 넣을 수 있는 동기화 계정 목록 [{id, name, color}]
 */
export function buildEditor({ event = null, defaults = {}, calendars = [], accounts = [], onSave, onDelete, onCancel }) {
  const today = toKey(new Date());
  const start = event?.startDate ?? defaults.startDate ?? today;
  const v = {
    title: event?.title ?? '',
    startDate: start,
    endDate: event?.endDate ?? defaults.endDate ?? start,
    allDay: event?.allDay ?? defaults.allDay ?? true,
    startTime: event?.startTime ?? defaults.startTime ?? '09:00',
    endTime: event?.endTime ?? defaults.endTime ?? '10:00',
    color: event?.color || COLORS[0],
    reminders: event?.reminders ?? [],
    memo: event?.memo ?? '',
    calendarId: event?.calendarId ?? 'local',
  };

  const form = document.createElement('form');
  form.className = 'editor';
  form.noValidate = true;
  form.innerHTML = `
    <label class="field"><span>제목</span>
      <input name="title" type="text" maxlength="200" autocomplete="off" placeholder="일정 제목">
    </label>
    <label class="field" data-role="cal-field" hidden><span>캘린더</span><select name="calendar"></select></label>
    <div class="row">
      <label class="field"><span>시작일</span><input name="startDate" type="date"></label>
      <label class="field"><span>종료일</span><input name="endDate" type="date"></label>
    </div>
    <label class="check"><input name="allDay" type="checkbox"><span>종일</span></label>
    <div class="row" data-role="times">
      <label class="field"><span>시작 시간</span><input name="startTime" type="time"></label>
      <label class="field"><span>종료 시간</span><input name="endTime" type="time"></label>
    </div>
    <div class="field"><span>색상</span><div class="swatches" data-role="swatches"></div></div>
    <div class="field"><span>알림 (여러 개 선택 가능)</span><div class="chips" data-role="reminders"></div></div>
    <label class="field"><span>메모</span><textarea name="memo" rows="3"></textarea></label>
    <div class="actions">
      <button type="button" class="btn danger" data-act="delete" hidden>삭제</button>
      <button type="button" class="btn" data-act="cancel">취소</button>
      <button type="submit" class="btn primary">저장</button>
    </div>`;

  const f = form.elements;
  f.title.value = v.title;
  f.startDate.value = v.startDate;
  f.endDate.value = v.endDate;
  f.allDay.checked = v.allDay;
  f.startTime.value = v.startTime;
  f.endTime.value = v.endTime;
  f.memo.value = v.memo;

  // 색상 선택
  let colorTouched = false;
  const swatches = form.querySelector('[data-role="swatches"]');
  const swatchInputs = [];
  for (const c of COLORS) {
    const label = document.createElement('label');
    label.className = 'sw';
    label.style.setProperty('--c', c);
    const input = document.createElement('input');
    input.type = 'radio';
    input.name = 'color';
    input.value = c;
    input.checked = c === v.color;
    input.addEventListener('change', () => (colorTouched = true));
    label.append(input, document.createElement('span'));
    swatches.append(label);
    swatchInputs.push(input);
  }

  // 알림 선택(칩). 서버에서 온 값처럼 목록에 없는 값도 보여 준다.
  const remBox = form.querySelector('[data-role="reminders"]');
  const choices = [...new Set([...REMINDER_CHOICES, ...v.reminders])].sort((a, b) => a - b);
  for (const m of choices) {
    const label = document.createElement('label');
    label.className = 'chipcheck';
    const input = document.createElement('input');
    input.type = 'checkbox';
    input.value = String(m);
    input.checked = v.reminders.includes(m);
    const span = document.createElement('span');
    span.textContent = reminderLabel(m);
    label.append(input, span);
    remBox.append(label);
  }

  // 캘린더 선택: 동기화 계정이 있을 때만 보인다. 이미 만든 일정은 옮길 수 없다.
  const calField = form.querySelector('[data-role="cal-field"]');
  const accountName = (id) => accounts.find((a) => a.id === id)?.name ?? '내 캘린더';
  if (event) {
    if (event.calendarId !== 'local') {
      calField.hidden = false;
      const o = document.createElement('option');
      o.textContent = accountName(event.calendarId);
      f.calendar.append(o);
      f.calendar.disabled = true;
    }
  } else if (calendars.length > 0) {
    calField.hidden = false;
    const local = document.createElement('option');
    local.value = 'local';
    local.textContent = '내 캘린더 (이 PC에만 저장)';
    f.calendar.append(local);
    for (const c of calendars) {
      const o = document.createElement('option');
      o.value = c.id;
      o.textContent = c.name;
      f.calendar.append(o);
    }
    f.calendar.value = defaults.calendarId && calendars.some((c) => c.id === defaults.calendarId) ? defaults.calendarId : 'local';
    f.calendar.addEventListener('change', () => {
      const acc = calendars.find((c) => c.id === f.calendar.value);
      if (acc && !colorTouched) {
        const target = swatchInputs.find((i) => i.value === acc.color);
        if (target) target.checked = true;
      }
    });
  }

  // 종일이면 시간 입력을 숨긴다.
  const times = form.querySelector('[data-role="times"]');
  const syncTimes = () => {
    times.hidden = f.allDay.checked;
  };
  f.allDay.addEventListener('change', syncTimes);
  syncTimes();

  // 시작일을 바꾸면 종료일이 더 빠르지 않게 맞춘다.
  f.startDate.addEventListener('change', () => {
    if (f.endDate.value < f.startDate.value) f.endDate.value = f.startDate.value;
  });
  f.title.addEventListener('input', () => f.title.classList.remove('bad'));

  // 삭제는 두 번 눌러야 실행된다.
  const delBtn = form.querySelector('[data-act="delete"]');
  if (event && onDelete) {
    delBtn.hidden = false;
    let armed = false;
    delBtn.addEventListener('click', () => {
      if (!armed) {
        armed = true;
        delBtn.textContent = '한 번 더 누르면 삭제';
        setTimeout(() => {
          armed = false;
          delBtn.textContent = '삭제';
        }, 3000);
        return;
      }
      onDelete(event);
    });
  }

  form.querySelector('[data-act="cancel"]').addEventListener('click', () => onCancel?.());

  form.addEventListener('submit', (ev) => {
    ev.preventDefault();
    const title = f.title.value.trim();
    if (!title) {
      f.title.classList.add('bad');
      f.title.focus();
      return;
    }
    const startDate = f.startDate.value || today;
    let endDate = f.endDate.value || startDate;
    if (endDate < startDate) endDate = startDate;

    const allDay = f.allDay.checked;
    const startTime = allDay ? null : f.startTime.value || '09:00';
    let endTime = allDay ? null : f.endTime.value || null;
    if (!allDay && startDate === endDate && endTime && endTime < startTime) endTime = startTime;

    const reminders = [...remBox.querySelectorAll('input:checked')].map((i) => Number(i.value)).sort((a, b) => a - b);
    onSave({
      ...(event ?? {}),
      id: event?.id ?? crypto.randomUUID(),
      title,
      startDate,
      endDate,
      allDay,
      startTime,
      endTime,
      memo: f.memo.value,
      color: form.querySelector('input[name="color"]:checked')?.value ?? COLORS[0],
      reminders,
      calendarId: event?.calendarId ?? (calField.hidden ? 'local' : f.calendar.value),
      updatedAt: Date.now(),
    });
  });

  return form;
}
