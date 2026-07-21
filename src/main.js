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
  Unknown:      { cls: "error",   text: "Служба недоступна" },
};

let current = "Unknown";
let busy = false;

async function refresh() {
  if (busy) return;
  const s = await invoke("nb_status");
  current = s.state;
  const v = VIEW[s.state] ?? VIEW.Unknown;

  el.toggle.className = `toggle ${v.cls}`;
  el.state.textContent = v.text;
  el.ip.textContent = s.state === "Connected" ? s.ip : "";
  el.url.value = s.mgmt_url;
}

el.toggle.onclick = async () => {
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
