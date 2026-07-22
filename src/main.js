const { invoke } = window.__TAURI__.tauri;

const el = {
  toggle: document.getElementById("toggle"),
  dot: document.getElementById("dot"),
  state: document.getElementById("state"),
  ip: document.getElementById("ip"),
  url: document.getElementById("url"),
  help: document.getElementById("help"),
  install: document.getElementById("install"),
  update: document.getElementById("update"),
  version: document.getElementById("version"),
};

const SPIN = '<span class="spinner"></span>';

// cls управляет цветом индикатора и тумблера; text — подпись состояния.
const VIEW = {
  Connected:    { cls: "on",      text: "Подключено" },
  Connecting:   { cls: "pending", text: "Подключение…" },
  NeedsLogin:   { cls: "pending", text: "Требуется вход" },
  Disconnected: { cls: "off",     text: "Отключено" },
  NotInstalled: { cls: "off",     text: "NetBird не установлен" },
  Installing:   { cls: "pending", text: "Устанавливается NetBird…" },
  InstallError: { cls: "error",   text: "Не удалось установить NetBird" },
  Unknown:      { cls: "error",   text: "Служба недоступна" },
};

let current = "Unknown";
let busy = false;          // выполняется up/down — не даём refresh перетереть состояние
let installing = false;    // установщик запущен и ещё не завершился
let installError = false;  // последняя попытка установки провалилась
let autoInstallTried = false;

async function refresh() {
  if (busy) return;
  let s;
  try {
    s = await invoke("nb_status");
  } catch {
    return; // временная недоступность бэкенда — пропускаем такт
  }
  current = s.state;

  // netbird появился → установка завершена
  if (s.state !== "NotInstalled") {
    installing = false;
    installError = false;
  }

  // netbird не найден → ставим сам, один раз за сессию (при ошибке — ручной повтор)
  if (s.state === "NotInstalled" && !autoInstallTried && !installing) {
    startInstall();
  }

  render(s);
}

async function startInstall() {
  autoInstallTried = true;
  installing = true;
  installError = false;
  render({ state: "NotInstalled" });
  try {
    await invoke("install_netbird"); // резолвится после закрытия установщика
    // фактический статус подтянет следующий refresh
  } catch {
    installing = false;
    installError = true;
  }
}

function render(s) {
  let key;
  if (installing) key = "Installing";
  else if (installError && s.state === "NotInstalled") key = "InstallError";
  else key = s.state;

  const v = VIEW[key] ?? VIEW.Unknown;

  el.dot.className = `dot ${v.cls}`;
  el.toggle.className = `toggle ${v.cls === "on" ? "on" : ""}`;

  if (v.cls === "pending") el.state.innerHTML = SPIN + v.text;
  else el.state.textContent = v.text;

  el.ip.textContent = s.state === "Connected" ? s.ip : "";
  if (s.mgmt_url) el.url.value = s.mgmt_url;

  // Тумблер доступен только когда есть чем управлять (netbird установлен).
  const canToggle = ["Connected", "Connecting", "NeedsLogin", "Disconnected"].includes(s.state);
  el.toggle.disabled = !canToggle;
  el.toggle.hidden = !canToggle;

  // Кнопка установки — когда netbird отсутствует и мы не в процессе установки.
  const showInstall = s.state === "NotInstalled" && !installing;
  el.install.hidden = !showInstall;
  el.install.textContent = installError ? "Повторить установку" : "Установить NetBird";
}

el.toggle.onclick = async () => {
  if (el.toggle.disabled) return;
  busy = true;
  el.dot.className = "dot pending";
  el.toggle.className = "toggle";
  el.state.innerHTML = SPIN + (current === "Connected" ? "Отключение…" : "Подключение…");
  try {
    if (current === "Connected") await invoke("nb_down");
    else await invoke("nb_up"); // при NeedsLogin netbird сам откроет браузер
  } catch {
    el.dot.className = "dot error";
    el.state.textContent = "Ошибка подключения";
  }
  busy = false;
  refresh();
};

el.install.onclick = () => {
  if (!installing) startInstall();
};

el.help.onclick = () => invoke("open_help");
el.update.onclick = () => invoke("open_download");

async function checkUpdate() {
  try {
    const u = await invoke("check_update");
    if (u.current) el.version.textContent = "Версия " + u.current;
    if (u.available) {
      el.update.querySelector(".update-text").textContent = `Доступна версия ${u.latest}`;
      el.update.hidden = false;
    }
  } catch {
    /* проверка обновления не критична */
  }
}

refresh();
checkUpdate();
setInterval(refresh, 2000);
