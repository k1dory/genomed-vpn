const { invoke } = window.__TAURI__.tauri;

const el = {
  toggle: document.getElementById("toggle"),
  state: document.getElementById("state"),
  ip: document.getElementById("ip"),
  url: document.getElementById("url"),
  help: document.getElementById("help"),
};

const VIEW = {
  Connected:    { cls: "on",      text: "Подключено" },
  Connecting:   { cls: "pending", text: "Подключение…" },
  NeedsLogin:   { cls: "pending", text: "Требуется вход" },
  Disconnected: { cls: "off",     text: "Отключено" },
  NotInstalled: { cls: "off",     text: "NetBird не установлен" },
  Installing:   { cls: "pending", text: "Устанавливается NetBird…" },
  Unknown:      { cls: "error",   text: "Служба недоступна" },
};

let current = "Unknown";
let busy = false;
let installing = false;
let autoInstallTried = false;

async function refresh() {
  if (busy) return;
  const s = await invoke("nb_status");
  current = s.state;

  // netbird появился → установка завершена
  if (s.state !== "NotInstalled") installing = false;

  // netbird не найден → ставим сам, один раз за сессию (Windows: качает и запускает установщик с UAC)
  if (s.state === "NotInstalled" && !autoInstallTried && !installing) {
    autoInstallTried = true;
    installing = true;
    invoke("install_netbird").catch(() => { installing = false; });
  }

  const key = installing && s.state === "NotInstalled" ? "Installing" : s.state;
  const v = VIEW[key] ?? VIEW.Unknown;

  el.toggle.className = `toggle ${v.cls}`;
  el.state.textContent = v.text;
  el.ip.textContent = s.state === "Connected" ? s.ip : "";
  el.url.value = s.mgmt_url;
  el.toggle.disabled = s.state === "NotInstalled";
}

el.toggle.onclick = async () => {
  if (current === "NotInstalled") return;
  busy = true;
  el.state.textContent = "…";
  try {
    if (current === "Connected") await invoke("nb_down");
    else await invoke("nb_up");   // при NeedsLogin netbird сам откроет браузер
  } catch (e) {
    el.state.textContent = "Ошибка";
  }
  busy = false;
  refresh();
};

el.help.onclick = () => invoke("open_help");

refresh();
setInterval(refresh, 2000);
