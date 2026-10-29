mod backup;
mod pin;
mod reminder;
mod store;
mod sync;
mod place;
mod toast;

use std::time::Duration;
use store::{Account, AccountInfo, Event, Settings, Store};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_window_state::{AppHandleExt as _, StateFlags};
use uuid::Uuid;

const PALETTE: [&str; 8] = [
    "#5b9dff", "#ff6b7a", "#3ec99a", "#f2a93b", "#a98bff", "#34c5d8", "#f272b0", "#8b94a8",
];

// ───────────────────────── 일정 ─────────────────────────

#[tauri::command]
fn list_events(store: State<'_, Store>) -> Vec<Event> {
    store.events()
}

#[tauri::command]
fn save_event(app: AppHandle, store: State<'_, Store>, event: Event) -> Result<(), String> {
    if event.calendar_id != "local" {
        match store.account(&event.calendar_id) {
            Some(a) if !a.read_only() => {}
            Some(_) => return Err("읽기 전용 캘린더에는 일정을 추가할 수 없습니다.".into()),
            None => return Err("연결되지 않은 캘린더입니다.".into()),
        }
    }
    let calendar = event.calendar_id.clone();
    let existing = store.raw_events().into_iter().find(|e| e.id == event.id).map(|e| e.calendar_id);
    store.upsert(event)?;
    let _ = app.emit("events-changed", ());
    let _ = backup::run(&app, false);
    // 기존 일정이면 원래 소속 계정, 새 일정이면 선택한 계정을 동기화한다.
    let target = existing.unwrap_or(calendar);
    if target != "local" {
        sync::schedule(&app, target);
    }
    Ok(())
}

#[tauri::command]
fn delete_event(app: AppHandle, store: State<'_, Store>, id: String) -> Result<(), String> {
    let calendar = store.raw_events().into_iter().find(|e| e.id == id).map(|e| e.calendar_id);
    store.remove(&id)?;
    let _ = app.emit("events-changed", ());
    let _ = backup::run(&app, false);
    if let Some(c) = calendar {
        if c != "local" {
            sync::schedule(&app, c);
        }
    }
    Ok(())
}

// ───────────────────────── 설정 ─────────────────────────

#[tauri::command]
fn get_settings(store: State<'_, Store>) -> Settings {
    store.settings()
}

#[tauri::command]
fn save_settings(app: AppHandle, store: State<'_, Store>, settings: Settings) -> Result<Settings, String> {
    let saved = store.set_settings(settings)?;
    let _ = app.emit("settings-changed", &saved);
    Ok(saved)
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let m = app.autolaunch();
    let r = if enabled { m.enable() } else { m.disable() };
    r.map_err(|e| e.to_string())
}

/// 알림 테스트. style을 주면 저장된 설정과 관계없이 그 방식("popup" | "windows")으로 보여 준다.
#[tauri::command]
fn test_notification(app: AppHandle, style: Option<String>) {
    let style = style.unwrap_or_else(|| app.state::<Store>().settings().notify_style);
    if style == "windows" {
        reminder::notify(&app, "알림 테스트", "일정 알림이 이렇게 표시됩니다.");
    } else {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        toast::show(&app, "알림 테스트", "일정 알림이 이렇게 표시됩니다.", "#5b9dff", &today);
    }
}

/// Google 로그인 정보(Client ID/Secret)를 이 PC의 데이터 폴더에 저장한다. 관리자가 최초 1회만 하면 되고,
/// 이후에는 모든 계정이 [Google로 로그인] 버튼만으로 연결된다.
#[tauri::command]
fn save_google_client(store: State<'_, Store>, client_id: String, client_secret: String) -> Result<(), String> {
    let (id, secret) = (client_id.trim().to_string(), client_secret.trim().to_string());
    if id.is_empty() || secret.is_empty() {
        return Err("Client ID와 Client Secret을 모두 입력해 주세요.".into());
    }
    if !id.ends_with(".apps.googleusercontent.com") {
        return Err("Client ID 형식이 올바르지 않습니다. (…apps.googleusercontent.com 으로 끝나야 합니다)".into());
    }
    store::write_json(
        &store.dir().join("google_client.json"),
        &serde_json::json!({ "clientId": id, "clientSecret": secret }),
    )
}

/// 사용자가 직접 추가한 공휴일·기념일(데이터 폴더의 holidays_custom.json). 파일이 없거나 형식이 틀리면 빈 목록.
/// 새로 지정되는 임시공휴일처럼 앱이 미리 알 수 없는 날짜를 넣는 용도다.
#[tauri::command]
fn get_custom_holidays(store: State<'_, Store>) -> serde_json::Value {
    std::fs::read_to_string(store.dir().join("holidays_custom.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v.is_array())
        .unwrap_or_else(|| serde_json::json!([]))
}

/// 데이터 폴더를 탐색기로 연다. holidays_custom.json이 없으면 빈 파일을 만들어 둔다.
#[tauri::command]
fn open_data_folder(store: State<'_, Store>) -> Result<(), String> {
    let dir = store.dir().to_path_buf();
    let file = dir.join("holidays_custom.json");
    if !file.exists() {
        let _ = std::fs::write(&file, "[]\n");
    }
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("xdg-open").arg(&dir).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// 이동 모드: 켜 두는 동안만 위젯을 옮기고 크기를 바꿀 수 있다. 끄면 위치를 저장하고 고정한다.
#[tauri::command]
fn set_move_mode(app: AppHandle, enabled: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_resizable(enabled);
    }
    if !enabled {
        let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
    }
}

// ───────────────────────── 창 ─────────────────────────

fn valid_date(s: &str) -> bool {
    s.len() == 10 && s.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// 날짜를 클릭했을 때 별도의 상세 창을 연다. 이미 열려 있으면 날짜만 바꿔서 앞으로 가져온다.
/// (Windows에서 창 생성은 async 명령에서 해야 멈춤 현상이 없다.)
#[tauri::command]
async fn open_detail(app: AppHandle, date: String) -> Result<(), String> {
    if !valid_date(&date) {
        return Err("날짜 형식이 올바르지 않습니다.".into());
    }
    if let Some(win) = app.get_webview_window("detail") {
        let _ = app.emit_to("detail", "detail-date", date);
        let _ = win.unminimize();
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(&app, "detail", WebviewUrl::App(format!("detail.html?date={date}").into()))
        .title("일정 상세")
        .inner_size(440.0, 680.0)
        .min_inner_size(380.0, 500.0)
        .theme(Some(tauri::Theme::Dark))
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    // 위젯이 있는 모니터 가운데에 띄운다(기본 모니터가 아니라).
    place::center_on_widget_monitor(&app, &win, 440.0, 680.0);
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

async fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.unminimize();
        win.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let win = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Widget Calendar 설정")
        .inner_size(600.0, 780.0)
        .min_inner_size(480.0, 560.0)
        .theme(Some(tauri::Theme::Dark))
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    place::center_on_widget_monitor(app, &win, 600.0, 780.0);
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

#[tauri::command]
async fn open_settings(app: AppHandle) -> Result<(), String> {
    open_settings_window(&app).await
}

/// 위젯을 보이게 한다. peek=true면 다른 창에 가려져 있어도 5초간 맨 앞으로 가져온다.
fn show_widget(app: &AppHandle, peek: bool) {
    let Some(w) = app.get_webview_window("main") else { return };
    pin::set_visible(true);
    let _ = w.show();
    let _ = w.unminimize();
    if peek {
        let _ = w.set_always_on_bottom(false);
        let _ = w.set_always_on_top(true);
        let w2 = w.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(5));
            let _ = w2.set_always_on_top(false);
            let _ = w2.set_always_on_bottom(true);
        });
    }
}

#[tauri::command]
fn hide_widget(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        pin::set_visible(false);
        let _ = w.hide();
    }
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "위젯 보이기", true, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync", "지금 동기화", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "설정", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &sync_item, &settings, &quit])?;

    let mut builder = TrayIconBuilder::new()
        .tooltip("Widget Calendar")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_widget(app, true),
            "sync" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = sync_all(&app, None).await;
                });
            }
            "settings" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = open_settings_window(&app).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_widget(tray.app_handle(), true);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ───────────────────────── 동기화 계정 ─────────────────────────

fn next_color(store: &Store) -> String {
    PALETTE[(store.accounts().len() + 1) % PALETTE.len()].to_string()
}

fn new_account_id() -> String {
    format!("acc-{}", &Uuid::new_v4().simple().to_string()[..12])
}

fn notify_accounts_changed(app: &AppHandle) {
    let _ = app.emit("accounts-changed", ());
    let _ = app.emit("events-changed", ());
}

#[tauri::command]
fn list_accounts(store: State<'_, Store>) -> Vec<AccountInfo> {
    store.accounts().iter().map(AccountInfo::from).collect()
}

/// 앱에 Google 로그인 정보가 들어 있어 [Google로 로그인] 버튼만으로 연결할 수 있는지
#[tauri::command]
fn google_login_available(store: State<'_, Store>) -> bool {
    sync::google::builtin_client(store.dir()).is_some()
}

/// client_id/client_secret을 모두 비워 보내면 앱에 내장된 로그인 정보로 연결한다.
#[tauri::command]
async fn add_google_account(
    app: AppHandle,
    name: String,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<(), String> {
    let (cid, secret) = (
        client_id.unwrap_or_default().trim().to_string(),
        client_secret.unwrap_or_default().trim().to_string(),
    );
    let manual = !cid.is_empty() || !secret.is_empty();
    if manual && (cid.is_empty() || secret.is_empty()) {
        return Err("Client ID와 Client Secret을 모두 입력해 주세요.".into());
    }
    let (use_id, use_secret) = if manual {
        (cid.clone(), secret.clone())
    } else {
        sync::google::builtin_client(app.state::<Store>().dir()).ok_or_else(|| {
            "이 앱에는 Google 로그인 정보가 설정되어 있지 않습니다. README의 'Google 로그인 설정'을 확인하세요.".to_string()
        })?
    };
    let refresh = sync::google::authorize(&use_id, &use_secret).await?;
    let store = app.state::<Store>();
    let acc = Account {
        id: new_account_id(),
        kind: "google".into(),
        name: if name.trim().is_empty() { "Google 캘린더".into() } else { name.trim().to_string() },
        color: next_color(&store),
        // 내장 정보를 쓴 계정은 비워 두고, 동기화할 때마다 앱의 값을 읽는다.
        client_id: if manual { cid } else { String::new() },
        client_secret: if manual { secret } else { String::new() },
        refresh_token: refresh,
        enabled: true,
        ..Default::default()
    };
    let id = acc.id.clone();
    store.add_account(acc)?;
    notify_accounts_changed(&app);
    sync::schedule(&app, id);
    Ok(())
}

#[tauri::command]
async fn add_caldav_account(
    app: AppHandle,
    name: String,
    preset: String,
    url: String,
    username: String,
    password: String,
) -> Result<(), String> {
    let url = if preset == "naver" { "https://caldav.calendar.naver.com".to_string() } else { url.trim().to_string() };
    let (username, password) = (username.trim().to_string(), password);
    if url.is_empty() || username.is_empty() || password.is_empty() {
        return Err("서버 주소, 아이디, 비밀번호를 모두 입력해 주세요.".into());
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("서버 주소는 https:// 로 시작해야 합니다.".into());
    }
    let default_name = if preset == "naver" { "네이버 캘린더" } else { "CalDAV 캘린더" };
    let store = app.state::<Store>();
    let mut acc = Account {
        id: new_account_id(),
        kind: "caldav".into(),
        name: if name.trim().is_empty() { default_name.into() } else { name.trim().to_string() },
        color: next_color(&store),
        url,
        username,
        password,
        enabled: true,
        ..Default::default()
    };
    // 연결 확인을 겸해 캘린더를 찾는다. 실패하면 계정을 만들지 않는다.
    let cals = sync::caldav::discover(&acc).await?;
    acc.calendars = cals.into_iter().map(|c| c.url).collect();
    let id = acc.id.clone();
    store.add_account(acc)?;
    notify_accounts_changed(&app);
    sync::schedule(&app, id);
    Ok(())
}

#[tauri::command]
async fn add_ics_account(app: AppHandle, name: String, url: String) -> Result<(), String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("ICS 주소 또는 파일 경로를 입력해 주세요.".into());
    }
    let text = sync::fetch_text(&url).await?;
    if !text.to_uppercase().contains("BEGIN:VCALENDAR") {
        return Err("올바른 캘린더(ICS) 형식이 아닙니다.".into());
    }
    let store = app.state::<Store>();
    let acc = Account {
        id: new_account_id(),
        kind: "ics".into(),
        name: if name.trim().is_empty() { "ICS 캘린더".into() } else { name.trim().to_string() },
        color: next_color(&store),
        url,
        enabled: true,
        ..Default::default()
    };
    let id = acc.id.clone();
    store.add_account(acc)?;
    notify_accounts_changed(&app);
    sync::schedule(&app, id);
    Ok(())
}

#[tauri::command]
fn remove_account(app: AppHandle, store: State<'_, Store>, id: String) -> Result<(), String> {
    store.remove_account(&id)?;
    notify_accounts_changed(&app);
    Ok(())
}

async fn sync_all(app: &AppHandle, only: Option<String>) -> Result<(), String> {
    let accounts = app.state::<Store>().accounts();
    let mut errors: Vec<String> = Vec::new();
    for a in accounts.into_iter().filter(|a| a.enabled && only.as_ref().map_or(true, |i| *i == a.id)) {
        if let Err(e) = sync::sync_and_notify(app, &a.id).await {
            errors.push(format!("{}: {}", a.name, e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[tauri::command]
async fn sync_now(app: AppHandle, id: Option<String>) -> Result<(), String> {
    sync_all(&app, id).await
}

// ───────────────────────── 백업 ─────────────────────────

#[tauri::command]
fn backup_now(app: AppHandle) -> Result<String, String> {
    let dir = backup::run(&app, true)?.ok_or_else(|| "백업을 만들지 못했습니다.".to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
fn backup_info(app: AppHandle) -> Result<backup::BackupInfo, String> {
    backup::info(&app)
}

#[tauri::command]
fn restore_backup(app: AppHandle) -> Result<usize, String> {
    let n = backup::restore(&app)?;
    let _ = app.emit("events-changed", ());
    Ok(n)
}

#[tauri::command]
fn open_backup_folder(app: AppHandle) -> Result<(), String> {
    backup::open_folder(&app)
}

// ───────────────────────── 시작 ─────────────────────────

pub fn run() {
    tauri::Builder::default()
        // 두 번째 실행은 새로 띄우지 않고 기존 위젯을 보여 준다. (가장 먼저 등록해야 한다.)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_widget(app, true);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        // 위젯의 위치와 크기를 기억한다. 상세·설정 창은 매번 가운데에 띄우므로 제외.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION | StateFlags::SIZE)
                .with_denylist(&["detail", "settings"])
                .build(),
        )
        .on_window_event(|window, event| match event {
            // 위젯의 닫기(Alt+F4 포함)는 종료가 아니라 트레이로 숨기기. 알림이 계속 동작해야 한다.
            WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
                api.prevent_close();
                pin::set_visible(false);
                let _ = window.hide();
            }
            WindowEvent::Destroyed => {
                let label = window.label().to_string();
                let app = window.app_handle();
                if label.starts_with("toast-") {
                    toast::on_destroyed(app, &label);
                } else if label == "settings" {
                    // 저장하지 않고 닫았다면 미리보기를 지우고 저장된 설정으로 되돌린다.
                    let saved = app.state::<Store>().settings();
                    let _ = app.emit("settings-changed", &saved);
                }
            }
            _ => {}
        })
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            app.manage(Store::load(dir));
            // 위젯은 평소에 고정(이동·크기 조절 불가). 메뉴의 '이동'을 켜야 움직인다.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_resizable(false);
            }
            let id = app.config().identifier.clone();
            std::thread::spawn(move || reminder::register_app_id(&id, "Widget Calendar"));
            build_tray(app.handle())?;
            pin::install(app.handle());
            reminder::spawn(app.handle().clone());
            sync::spawn_auto(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_events,
            save_event,
            delete_event,
            get_settings,
            save_settings,
            get_autostart,
            set_autostart,
            test_notification,
            set_move_mode,
            get_custom_holidays,
            save_google_client,
            open_data_folder,
            open_detail,
            open_settings,
            hide_widget,
            quit_app,
            list_accounts,
            google_login_available,
            add_google_account,
            add_caldav_account,
            add_ics_account,
            remove_account,
            sync_now,
            backup_now,
            backup_info,
            restore_backup,
            open_backup_folder
        ])
        .run(tauri::generate_context!())
        .expect("앱 실행 중 오류가 발생했습니다.");
}
