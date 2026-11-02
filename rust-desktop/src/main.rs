#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod instance;
mod job_gate;
mod portable_fingerprint;
mod runtime_setup;
mod service;
mod shell;
mod updater;
use serde_json::{json, Value};
use service::{Options, Service, VERSION};
use std::{path::PathBuf, sync::Arc};
use tauri::{Manager, State};

fn options() -> Result<Options, String> {
    let mut data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("사용자 설정 폴더가 없습니다.")?
        .join("ProjectConversationManager");
    let mut home = None;
    let mut health = None;
    let mut no_update = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state-dir" | "-s" => data = PathBuf::from(args.next().ok_or("설정 폴더 경로 누락")?),
            "--home" => home = Some(PathBuf::from(args.next().ok_or("저장소 경로 누락")?)),
            "--update-health" => {
                health = Some(PathBuf::from(args.next().ok_or("실행 확인 경로 누락")?))
            }
            "--no-update-check" => no_update = true,
            _ => return Err(format!("알 수 없는 실행 옵션: {arg}")),
        }
    }
    let test_root = std::env::var_os("CODEX_MANAGER_TEST_ROOT").map(PathBuf::from);
    Ok(Options {
        data,
        home,
        health,
        no_update,
        test_root,
    })
}

#[tauri::command]
async fn bootstrap(
    service: State<'_, Arc<Service>>,
    pane: Option<String>,
) -> Result<Value, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.bootstrap_pane(pane.as_deref()))
        .await
        .map_err(|_| "프로그램 정보를 읽지 못했습니다.")?
}
#[tauri::command]
async fn catalog(service: State<'_, Arc<Service>>, home: String) -> Result<Value, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.catalog(&home))
        .await
        .map_err(|_| "목록을 읽지 못했습니다.")?
}
#[tauri::command]
async fn select_home(
    service: State<'_, Arc<Service>>,
    home: String,
    pane: Option<String>,
) -> Result<(), String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.select_home(&home, pane.as_deref()))
        .await
        .map_err(|_| "저장소를 연결하지 못했습니다.")?
}
#[tauri::command]
async fn start_job(
    service: State<'_, Arc<Service>>,
    action: String,
    args: Value,
) -> Result<String, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.start(action, args))
        .await
        .map_err(|_| "작업을 시작하지 못했습니다.")?
}
#[tauri::command]
fn job_status(service: State<'_, Arc<Service>>, id: String) -> Result<Value, String> {
    service.job(&id)
}
#[tauri::command]
fn cancel_job(service: State<'_, Arc<Service>>, id: String) -> Result<(), String> {
    service.cancel(&id)
}
#[tauri::command]
async fn history(service: State<'_, Arc<Service>>) -> Result<Vec<Value>, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.history())
        .await
        .map_err(|_| "작업 내역을 읽지 못했습니다.")?
}
#[tauri::command]
async fn file_list(
    service: State<'_, Arc<Service>>,
    home: String,
    projects: Vec<String>,
    directory: String,
) -> Result<Vec<Value>, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.files(&home, &projects, &directory))
        .await
        .map_err(|_| "파일 목록을 읽지 못했습니다.")?
}
#[tauri::command]
async fn transcript(
    service: State<'_, Arc<Service>>,
    home: String,
    id: String,
) -> Result<Value, String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.transcript(&home, &id))
        .await
        .map_err(|_| "대화를 읽지 못했습니다.")?
}
#[tauri::command]
async fn open_file(
    service: State<'_, Arc<Service>>,
    home: String,
    projects: Vec<String>,
    path: String,
) -> Result<(), String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.open_file(&home, &projects, &path))
        .await
        .map_err(|_| "파일을 열지 못했습니다.")?
}
#[tauri::command]
async fn choose_folder(initial: Option<String>) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_title("폴더 선택");
        if let Some(initial) = initial {
            let path = PathBuf::from(initial);
            if path.is_dir() {
                dialog = dialog.set_directory(path);
            }
        }
        dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().to_string())
    })
    .await
    .map_err(|_| "폴더 선택 창을 열지 못했습니다.".into())
}
#[tauri::command]
async fn apply_update(
    service: State<'_, Arc<Service>>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let s = Arc::clone(&service);
    tauri::async_runtime::spawn_blocking(move || s.apply_update())
        .await
        .map_err(|_| "업데이트를 시작하지 못했습니다.")??;
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn quit(service: State<'_, Arc<Service>>, app: tauri::AppHandle) -> Result<(), String> {
    if !service.can_close() {
        return Err("진행 중인 작업을 완료하거나 취소한 뒤 종료하세요.".into());
    }
    app.exit(0);
    Ok(())
}
#[tauri::command]
fn ready(service: State<'_, Arc<Service>>) -> Result<(), String> {
    let status = service
        .shell_status
        .lock()
        .map_err(|_| "창 상태 잠금 오류")?
        .clone();
    status.clone()?;
    if let Some(path) = &service.options.health {
        codex_manager_core::absolute(path)?;
        if !path.starts_with(service.options.data.join("updates")) {
            return Err("실행 확인 경로가 올바르지 않습니다.".into());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let status = service
            .shell_status
            .lock()
            .map_err(|_| "창 상태 잠금 오류")?
            .clone();
        codex_manager_core::write_json_new(
            path,
            &json!({"version":VERSION,"install_dir":exe.parent(),"pid":std::process::id(),"taskbar_registered":status.is_ok(),"taskbar_error":status.err()}),
        )?;
    }
    Ok(())
}

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("Codex Manager {VERSION}");
        return;
    }
    let service = match options().and_then(Service::new) {
        Ok(service) => Arc::new(service),
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("Codex Manager")
                .set_description(error)
                .set_level(rfd::MessageLevel::Error)
                .show();
            return;
        }
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let runtime = parent.join("_internal/WebView2");
            if runtime.join("msedgewebview2.exe").is_file() {
                if let Err(error) = runtime_setup::prepare(&runtime, &service.options.data) {
                    rfd::MessageDialog::new()
                        .set_title("Codex Manager 실행 준비")
                        .set_description(format!(
                            "{error}\n프로그램 폴더의 실행 권한을 확인한 뒤 다시 실행하세요."
                        ))
                        .show();
                    return;
                }
                std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", runtime);
            }
        }
    }
    if service.options.test_root.is_none() {
        std::env::set_var(
            "WEBVIEW2_USER_DATA_FOLDER",
            service.options.data.join("webview"),
        );
    }
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
        }
        let id = "Contentrium.CodexManager"
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        unsafe {
            let result = SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
            if result < 0 {
                eprintln!("Windows application identity error: {result}");
            }
        }
    }
    tauri::Builder::default()
        .manage(service)
        .setup(|app| {
            let service = app.state::<Arc<Service>>();
            service.start_automatic_loop();
            if let Some(window) = app.get_webview_window("main") {
                if let Some(monitor) = window.current_monitor()? {
                    let scale = monitor.scale_factor();
                    let area = monitor.work_area();
                    let width = (area.size.width as f64 / scale - 24.0).clamp(360.0, 1280.0);
                    let height = (area.size.height as f64 / scale - 40.0).clamp(360.0, 820.0);
                    window.set_min_size(Some(tauri::LogicalSize::new(
                        width.min(760.0),
                        height.min(520.0),
                    )))?;
                    window.set_size(tauri::LogicalSize::new(width, height))?;
                }
                #[cfg(windows)]
                {
                    let result =
                        std::env::current_exe()
                            .map_err(|e| e.to_string())
                            .and_then(|exe| {
                                shell::register(
                                    window.hwnd().map_err(|e| e.to_string())?,
                                    &exe,
                                    &service.options.data,
                                )
                            });
                    *service
                        .shell_status
                        .lock()
                        .map_err(|_| "창 상태 잠금 오류")? = result;
                }
            }
            if app.state::<Arc<Service>>().options.test_root.is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    window.set_focusable(false)?;
                }
            }
            use tauri::menu::{Menu, MenuItem};
            let open = MenuItem::with_id(app, "open", "창 열기", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "exit", "완전 종료", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &exit])?;
            let mut tray = tauri::tray::TrayIconBuilder::new()
                .tooltip("Codex Manager")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                        }
                    }
                    "exit" if app.state::<Arc<Service>>().can_close() => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if !window.state::<Arc<Service>>().can_close() {
                    api.prevent_close();
                    use tauri::Emitter;
                    let _ = window.emit(
                        "close-blocked",
                        "진행 중인 작업을 완료하거나 취소한 뒤 종료하세요.",
                    );
                } else {
                    #[cfg(windows)]
                    if let Ok(hwnd) = window.hwnd() {
                        let _ = shell::clear(hwnd);
                    }
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            catalog,
            select_home,
            start_job,
            job_status,
            cancel_job,
            history,
            file_list,
            transcript,
            open_file,
            choose_folder,
            ready,
            apply_update,
            quit
        ])
        .run(tauri::generate_context!())
        .expect("Codex Manager could not start");
}

#[cfg(test)]
mod window_config_tests {
    #[test]
    fn runtime_version_agrees_with_embedded_release_metadata() {
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        assert_eq!(
            context.config().version.as_deref(),
            Some(crate::service::VERSION)
        );
    }

    #[test]
    fn windows_frontend_drag_drop_is_not_intercepted_by_the_native_handler() {
        let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let window = &context.config().app.windows[0];
        assert!(
            !window.drag_drop_enabled,
            "WebView2 native drag interception prevents the explorer's HTML5 drag and drop"
        );
    }
}
