#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use serde::Serialize;
use std::process::Command;
use tauri::{
    CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
};

const MGMT_URL: &str = "https://vpn.genomed-security.ru";
const HELP_URL: &str = "https://help.genomed-security.ru";

#[derive(Serialize)]
struct Status {
    state: String,       // Connected | Connecting | NeedsLogin | Disconnected | Unknown
    ip: String,
    mgmt_url: String,
    mgmt_ok: bool,
}

#[cfg(target_os = "windows")]
fn cmd(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    Command::new("netbird")
        .args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW — без мигания консоли
        .output()
}

#[cfg(not(target_os = "windows"))]
fn cmd(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("netbird").args(args).output()
}

#[tauri::command]
fn nb_status() -> Status {
    let out = match cmd(&["status", "-j"]) {
        Ok(o) => o,
        Err(_) => {
            return Status {
                state: "Unknown".into(),
                ip: String::new(),
                mgmt_url: MGMT_URL.into(),
                mgmt_ok: false,
            }
        }
    };

    let txt = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap_or_default();

    let raw = v["daemonStatus"].as_str().unwrap_or("Unknown");
    let state = match raw {
        "Connected" => "Connected",
        "Connecting" => "Connecting",
        "NeedsLogin" | "LoginFailed" | "SessionExpired" => "NeedsLogin",
        "Disconnected" | "Idle" => "Disconnected",
        _ => "Unknown",
    };

    Status {
        state: state.into(),
        ip: v["netbirdIp"].as_str().unwrap_or("").to_string(),
        mgmt_url: v["management"]["url"].as_str().unwrap_or(MGMT_URL).to_string(),
        mgmt_ok: v["management"]["connected"].as_bool().unwrap_or(false),
    }
}

#[tauri::command]
fn nb_up() -> Result<(), String> {
    cmd(&["up", "--management-url", MGMT_URL])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn nb_down() -> Result<(), String> {
    cmd(&["down"]).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_help(app: tauri::AppHandle) {
    let _ = tauri::api::shell::open(&app.shell_scope(), HELP_URL, None);
}

fn main() {
    let tray_menu = SystemTrayMenu::new()
        .add_item(CustomMenuItem::new("show", "Открыть"))
        .add_item(CustomMenuItem::new("connect", "Подключить"))
        .add_item(CustomMenuItem::new("disconnect", "Отключить"))
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(CustomMenuItem::new("help", "Помощь"))
        .add_item(CustomMenuItem::new("quit", "Выход"));

    tauri::Builder::default()
        .system_tray(SystemTray::new().with_menu(tray_menu))
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick { .. } => {
                if let Some(w) = app.get_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "show" => {
                    if let Some(w) = app.get_window("main") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "connect" => { let _ = nb_up(); }
                "disconnect" => { let _ = nb_down(); }
                "help" => { let _ = tauri::api::shell::open(&app.shell_scope(), HELP_URL, None); }
                "quit" => std::process::exit(0),
                _ => {}
            },
            _ => {}
        })
        .on_window_event(|e| {
            // крестик прячет в трей, а не закрывает
            if let tauri::WindowEvent::CloseRequested { api, .. } = e.event() {
                e.window().hide().unwrap();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![nb_status, nb_up, nb_down, open_help])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
