//! Wiring: in-process core, IPC commands, window, tray, single instance, navigation guard, background loop.
//! Nothing here binds a socket.

use std::sync::OnceLock;

use axum::Router;
use corvus_core::background::checkpoint;
use corvus_core::{Config, Corvus};
use serde_json::Value;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Runtime, State, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent, Wry};

use crate::background::Background;
use crate::info::{self, AppInfo};
use crate::ipc::{ApiResponse, dispatch};
use crate::nav::{hand_to_browser, navigation_allowed};

const MAIN: &str = "main";
const PAUSE_LABEL: &str = "Pause background work";
const RESUME_LABEL: &str = "Resume background work";

struct AppState {
    router: Router,
    corvus: Corvus,
    background: Background,
    pause_item: OnceLock<MenuItem<Wry>>,
}

#[tauri::command]
async fn api(
    state: State<'_, AppState>,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<ApiResponse, String> {
    dispatch(&state.router, &method, &path, body).await
}

#[tauri::command]
fn app_info() -> AppInfo {
    info::gather_from_env()
}

#[tauri::command]
fn install_cli() -> Result<String, String> {
    info::install_cli_from_env()
}

#[tauri::command]
fn set_background_paused(app: AppHandle, paused: bool) {
    apply_paused(&app, paused);
}

fn apply_paused(app: &AppHandle, paused: bool) {
    let state = app.state::<AppState>();
    state.background.set_paused(paused);
    if let Some(item) = state.pause_item.get() {
        let _ = item.set_text(if paused { RESUME_LABEL } else { PAUSE_LABEL });
    }
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    // The close button hides the window; the app keeps running (and stays the background leader).
    if window.label() == MAIN
        && let WindowEvent::CloseRequested { api, .. } = event
    {
        api.prevent_close();
        let _ = window.hide();
    }
}

fn fail(message: &str) -> ! {
    eprintln!("corvus-desktop: {message}");
    std::process::exit(1)
}

/// Runs the app until it quits.
pub fn run(context: tauri::Context<Wry>) {
    let config = Config::load().unwrap_or_else(|e| fail(&format!("configuration error: {e}")));
    corvus_core::logging::init(&config.log);
    if let Err(e) = config.ensure_state_dir() {
        fail(&format!("cannot create the state directory: {e}"));
    }
    let (corvus, _report) =
        Corvus::open(&config).unwrap_or_else(|e| fail(&format!("cannot open the database: {e}")));
    let state = AppState {
        router: corvus_http::data_router(corvus.clone()),
        background: Background::new(corvus.services().clone(), config.state_dir.clone()),
        corvus,
        pause_item: OnceLock::new(),
    };

    let app = tauri::Builder::default()
        // Must be registered first: a second launch focuses the existing window and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            api,
            app_info,
            install_cli,
            set_background_paused
        ])
        .on_window_event(on_window_event)
        .setup(|app| {
            WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
                .title("Corvus")
                .inner_size(1200.0, 800.0)
                .on_navigation(|url| {
                    if navigation_allowed(url) {
                        true
                    } else {
                        hand_to_browser(url);
                        false
                    }
                })
                .on_new_window(|url, _features| {
                    hand_to_browser(&url);
                    NewWindowResponse::Deny
                })
                .build()?;
            build_tray(app.handle())?;
            app.state::<AppState>().background.set_paused(false);
            Ok(())
        })
        .build(context)
        .unwrap_or_else(|e| fail(&format!("cannot start the app: {e}")));

    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => show_main(app),
        tauri::RunEvent::Exit => {
            let state = app.state::<AppState>();
            tauri::async_runtime::block_on(state.background.shutdown());
            if let Err(e) = checkpoint(state.corvus.services()) {
                tracing::warn!(error = %e, "checkpoint on exit failed");
            }
        }
        _ => {}
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", PAUSE_LABEL, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &PredefinedMenuItem::separator(app)?, &quit])?;
    let _ = app.state::<AppState>().pause_item.set(pause);
    TrayIconBuilder::new()
        .icon(Image::from_bytes(include_bytes!("../icons/icon.png"))?)
        .tooltip("Corvus")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "pause" => {
                let paused = app.state::<AppState>().background.is_paused();
                apply_paused(app, !paused);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
