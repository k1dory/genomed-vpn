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
    state: String,       // Connected | Connecting | NeedsLogin | Disconnected | NotInstalled | Unknown
    ip: String,
    mgmt_url: String,
    mgmt_ok: bool,
}

/// Абсолютный путь к бинарю netbird, если он установлен, иначе имя для поиска в PATH.
/// Абсолютный путь важен: после установки уже запущенный процесс не видит обновлённый PATH,
/// а файл по фиксированному пути находится сразу — без перезапуска приложения.
fn netbird_exe() -> String {
    #[cfg(target_os = "windows")]
    {
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Ok(pf) = std::env::var(var) {
                let p = format!("{pf}\\NetBird\\netbird.exe");
                if std::path::Path::new(&p).exists() {
                    return p;
                }
            }
        }
        "netbird".into()
    }
    #[cfg(not(target_os = "windows"))]
    {
        for p in [
            "/usr/local/bin/netbird",
            "/usr/bin/netbird",
            "/opt/homebrew/bin/netbird",
        ] {
            if std::path::Path::new(p).exists() {
                return p.into();
            }
        }
        "netbird".into()
    }
}

#[cfg(target_os = "windows")]
fn cmd(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    Command::new(netbird_exe())
        .args(args)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW — без мигания консоли
        .output()
}

#[cfg(not(target_os = "windows"))]
fn cmd(args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(netbird_exe()).args(args).output()
}

#[tauri::command]
fn nb_status() -> Status {
    let out = match cmd(&["status", "-j"]) {
        Ok(o) => o,
        Err(_) => {
            // не удалось запустить бинарь netbird → он не установлен
            return Status {
                state: "NotInstalled".into(),
                ip: String::new(),
                mgmt_url: MGMT_URL.into(),
                mgmt_ok: false,
            };
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
        // всегда показываем НАШ сервер: приложение работает только с ним,
        // а netbird до первого переключения докладывает свой дефолт (api.netbird.io)
        mgmt_url: MGMT_URL.into(),
        mgmt_ok: v["management"]["connected"].as_bool().unwrap_or(false),
    }
}

#[tauri::command]
fn nb_up() -> Result<(), String> {
    let out = cmd(&["up", "--management-url", MGMT_URL]).map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    // netbird мог быть настроен на другой management (по умолчанию api.netbird.io)
    // и отказаться менять его на лету — сбрасываем и переподключаемся на наш сервер
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    if err.contains("management") || err.contains("url") || err.contains("differ") {
        let _ = cmd(&["down"]);
        let out2 = cmd(&["up", "--management-url", MGMT_URL]).map_err(|e| e.to_string())?;
        if out2.status.success() {
            return Ok(());
        }
        return Err(String::from_utf8_lossy(&out2.stderr).trim().to_string());
    }
    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
}

#[tauri::command]
fn nb_down() -> Result<(), String> {
    cmd(&["down"]).map(|_| ()).map_err(|e| e.to_string())
}

/// Bootstrap: скачивает официальный установщик NetBird и запускает его.
/// Windows — полный автомат (скачивание + запуск с правами администратора, UAC).
/// macOS   — скачивает .pkg и открывает штатный установщик.
/// Linux   — открывает официальную страницу установки (нужен sudo/скрипт).
#[tauri::command]
fn install_netbird() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // одной командой: тихо скачиваем во %TEMP% и запускаем с UAC (Start-Process -Verb RunAs)
        let ps = r#"$ProgressPreference='SilentlyContinue'; $u='https://pkgs.netbird.io/windows/x64'; $o=Join-Path $env:TEMP 'netbird_installer.exe'; Invoke-WebRequest -Uri $u -OutFile $o -UseBasicParsing; Start-Process -FilePath $o -Verb RunAs"#;
        let out = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps])
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { "amd64" };
        let url = format!("https://pkgs.netbird.io/macos/{arch}");
        let dest = "/tmp/netbird_installer.pkg";
        Command::new("curl")
            .args(["-fsSL", "-o", dest, &url])
            .output()
            .map_err(|e| e.to_string())?;
        // открывает штатный macOS Installer.app
        Command::new("open")
            .arg(dest)
            .output()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    #[cfg(target_os = "linux")]
    {
        // на Linux установка идёт скриптом с sudo — открываем инструкцию
        Command::new("xdg-open")
            .arg("https://docs.netbird.io/how-to/getting-started#installation")
            .output()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
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
        .invoke_handler(tauri::generate_handler![
            nb_status,
            nb_up,
            nb_down,
            install_netbird,
            open_help
        ])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
