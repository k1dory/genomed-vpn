#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use serde::Serialize;
use std::process::Command;
use tauri::{
    CustomMenuItem, Manager, SystemTray, SystemTrayEvent, SystemTrayMenu, SystemTrayMenuItem,
};

const MGMT_URL: &str = "https://vpn.genomed-security.ru";
const HELP_URL: &str = "https://help.genomed-security.ru";
const DOWNLOAD_URL: &str = "https://github.com/k1dory/genomed-vpn/releases/latest";
const RELEASE_API: &str = "https://api.github.com/repos/k1dory/genomed-vpn/releases/latest";

// Сколько ждём завершения `netbird up`: при SSO команда блокируется, пока
// пользователь не пройдёт вход в браузере. Щедрый лимит, чтобы не обрывать логин.
const UP_TIMEOUT_SECS: u64 = 180;

#[derive(Serialize)]
struct Status {
    state: String, // Connected | Connecting | NeedsLogin | Disconnected | NotInstalled | Unknown
    ip: String,
    mgmt_url: String,
    mgmt_ok: bool,
}

#[derive(Serialize)]
struct UpdateInfo {
    current: String,
    latest: String,
    available: bool,
    url: String,
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

/// Короткая команда netbird с ожиданием результата (status/down).
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

/// Команда netbird с таймаутом. Нужна для `up`: при SSO она блокируется до входа,
/// и без ограничения интерфейс завис бы навсегда, если пользователь не завершил логин.
fn cmd_timeout(args: &[&str], secs: u64) -> std::io::Result<std::process::Output> {
    use std::process::Stdio;
    let mut c = Command::new(netbird_exe());
    c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    let mut child = c.spawn()?;
    let start = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if start.elapsed().as_secs() >= secs {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "истекло время ожидания входа",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// HTTP GET через системный инструмент (без тяжёлых сетевых зависимостей).
/// Используется только для проверки обновлений; при любой ошибке возвращает None.
fn http_get(url: &str) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let ps = format!(
            "$ProgressPreference='SilentlyContinue'; \
             (Invoke-WebRequest -Uri '{url}' \
             -Headers @{{'User-Agent'='genomed-vpn'}} -UseBasicParsing -TimeoutSec 6).Content"
        );
        let out = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
            .creation_flags(0x08000000)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let out = Command::new("curl")
            .args([
                "-fsSL",
                "--max-time",
                "6",
                "-H",
                "User-Agent: genomed-vpn",
                url,
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

/// Сравнение версий вида `a.b.c` — true, если a строго новее b.
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.split('.').map(|x| x.parse().unwrap_or(0)).collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..3 {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
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
    let out = cmd_timeout(&["up", "--management-url", MGMT_URL], UP_TIMEOUT_SECS)
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    // netbird мог быть настроен на другой management (по умолчанию api.netbird.io)
    // и отказаться менять его на лету. Безопасно сбрасываем и переподключаемся на наш
    // сервер — без хрупкого разбора текста ошибки: down идемпотентен, повторный up
    // либо поднимает туннель, либо запускает SSO-вход.
    let first_err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let _ = cmd(&["down"]);
    let out2 = cmd_timeout(&["up", "--management-url", MGMT_URL], UP_TIMEOUT_SECS)
        .map_err(|e| e.to_string())?;
    if out2.status.success() {
        return Ok(());
    }
    let second_err = String::from_utf8_lossy(&out2.stderr).trim().to_string();
    Err(if second_err.is_empty() { first_err } else { second_err })
}

#[tauri::command]
fn nb_down() -> Result<(), String> {
    cmd(&["down"]).map(|_| ()).map_err(|e| e.to_string())
}

/// Bootstrap: скачивает официальный установщик NetBird и запускает его.
/// Windows — полный автомат: скачивание, проверка цифровой подписи, запуск с UAC
///           и ожидание завершения установщика.
/// macOS   — скачивает .pkg (с проверкой успеха загрузки) и открывает штатный установщик.
/// Linux   — открывает официальную страницу установки (нужен sudo/скрипт).
#[tauri::command]
fn install_netbird() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Скачиваем во %TEMP%, ПРОВЕРЯЕМ Authenticode-подпись установщика (защита от
        // подмены при MITM/компрометации зеркала) и только затем запускаем с UAC.
        // -Wait: PowerShell дожидается закрытия установщика, поэтому отмена UAC или
        // сбой установки вернутся сюда ненулевым кодом и превратятся в Err — интерфейс
        // не зависнет в состоянии «устанавливается».
        let ps = r#"
$ErrorActionPreference='Stop';
$ProgressPreference='SilentlyContinue';
$u='https://pkgs.netbird.io/windows/x64';
$o=Join-Path $env:TEMP 'netbird_installer.exe';
Invoke-WebRequest -Uri $u -OutFile $o -UseBasicParsing;
$sig=Get-AuthenticodeSignature $o;
if ($sig.Status -ne 'Valid') { Write-Error ('Недействительная подпись установщика NetBird: ' + $sig.Status); exit 1 }
Start-Process -FilePath $o -Verb RunAs -Wait
"#;
        let out = Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", ps])
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            "установка отменена или не завершена".into()
        } else {
            err
        });
    }

    #[cfg(target_os = "macos")]
    {
        let arch = if std::env::consts::ARCH == "aarch64" {
            "arm64"
        } else {
            "amd64"
        };
        let url = format!("https://pkgs.netbird.io/macos/{arch}");
        let dest = "/tmp/netbird_installer.pkg";
        let dl = Command::new("curl")
            .args(["-fsSL", "-o", dest, &url])
            .output()
            .map_err(|e| e.to_string())?;
        if !dl.status.success() {
            return Err("не удалось скачать установщик NetBird".into());
        }
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

/// Проверка обновления самого приложения через GitHub Releases.
/// Сеть недоступна или ошибка → available=false (тихо, без падения).
#[tauri::command]
fn check_update() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let latest = http_get(RELEASE_API)
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|v| {
            v["tag_name"]
                .as_str()
                .map(|s| s.trim_start_matches('v').to_string())
        })
        .unwrap_or_default();
    let available = !latest.is_empty() && version_gt(&latest, &current);
    UpdateInfo {
        current,
        latest,
        available,
        url: DOWNLOAD_URL.into(),
    }
}

#[tauri::command]
fn open_download(app: tauri::AppHandle) {
    let _ = tauri::api::shell::open(&app.shell_scope(), DOWNLOAD_URL, None);
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
                "connect" => {
                    let _ = nb_up();
                }
                "disconnect" => {
                    let _ = nb_down();
                }
                "help" => {
                    let _ = tauri::api::shell::open(&app.shell_scope(), HELP_URL, None);
                }
                "quit" => std::process::exit(0),
                _ => {}
            },
            _ => {}
        })
        .on_window_event(|e| {
            // крестик прячет в трей, а не закрывает
            if let tauri::WindowEvent::CloseRequested { api, .. } = e.event() {
                let _ = e.window().hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            nb_status,
            nb_up,
            nb_down,
            install_netbird,
            check_update,
            open_download,
            open_help
        ])
        .run(tauri::generate_context!())
        .expect("error while running application");
}
