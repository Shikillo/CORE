// Core — lógica de la interfaz. El backend crea/gestiona los webviews de
// las pestañas; aquí se mandan commands y se reciben avisos (core*) que el
// backend ejecuta con eval sobre esta vista.

"use strict";

const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);

async function call(cmd, args = {}) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    console.error(cmd, e);
  }
}

// --- Estado de pestañas (label → {url, title, loading}) -----------------------

const tabs = new Map();
let active = null;

// La casa: papel vacío (una data: URL con el color del papel), nada que enseñar.
const isBlank = (url) => !url || url === "about:blank" || url.startsWith("data:");

function tabLabel(info) {
  if (info.title) return info.title;
  if (isBlank(info.url)) return "en blanco";
  try { return new URL(info.url).host || info.url; } catch { return info.url; }
}

function renderTabs() {
  const list = $("tab-list");
  list.innerHTML = "";
  for (const [label, info] of tabs) {
    const li = document.createElement("li");
    li.classList.toggle("selected", label === active);
    const title = document.createElement("span");
    title.className = "tab-title";
    title.textContent = tabLabel(info);
    li.title = isBlank(info.url) ? "" : info.url;
    const close = document.createElement("button");
    close.className = "tab-close";
    close.title = "cerrar pestaña";
    close.textContent = "✕";
    close.addEventListener("click", (e) => {
      e.stopPropagation(); // que no seleccione la pestaña de paso
      call("close_tab", { label });
    });
    li.append(title, close);
    li.addEventListener("click", () => {
      if (label !== active) {
        setActive(label);
        call("select_tab", { label });
      }
    });
    list.appendChild(li);
  }
}

function setActive(label) {
  active = label;
  const info = tabs.get(label);
  if (info && document.activeElement !== $("url")) {
    $("url").value = isBlank(info.url) ? "" : info.url;
  }
  document.body.classList.toggle("loading", !!info?.loading);
  renderTabs();
}

// --- El hueco del navegador -------------------------------------------------------
//
// La UI es quien sabe dónde quedó el bloque «navegador» tras el layout de CSS:
// se mide su rectángulo (menos un respiro para que se vea el borde) y se
// comunica al backend, que coloca ahí el webview de la pestaña activa.

const INSET = 8;

function reportHole() {
  const r = $("blk-browser").getBoundingClientRect();
  call("set_hole", {
    x: r.left + INSET,
    y: r.top + INSET,
    w: r.width - INSET * 2,
    h: r.height - INSET * 2,
  });
}

new ResizeObserver(reportHole).observe($("blk-browser"));
window.addEventListener("resize", reportHole);
reportHole();

// --- Controles ------------------------------------------------------------------

$("go-back").addEventListener("click", () => call("nav_back"));
$("go-forward").addEventListener("click", () => call("nav_forward"));
$("go-reload").addEventListener("click", () => call("nav_reload"));
$("go-home").addEventListener("click", () => call("nav_home"));
$("tab-new").addEventListener("click", () => call("new_tab"));

$("url").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    call("navigate", { url: $("url").value });
    $("url").blur();
  } else if (e.key === "Escape") {
    $("url").blur();
  }
});
$("url").addEventListener("focus", () => $("url").select());

// --- Barra de carga (bajo las pestañas) -----------------------------------------------
//
// El webview no cuenta su progreso real: se estima con una curva asintótica
// (rápida al principio, se frena hacia el 90%) y al terminar la carga de
// verdad se remata al 100% un instante antes de vaciarse. Muestra siempre la
// pestaña activa.

const DONE_MS = 400; // cuánto luce el 100% antes de vaciarse

function drawProgress() {
  const info = tabs.get(active);
  const fill = $("load-fill");
  if (info?.loading && info.loadStart) {
    const t = (Date.now() - info.loadStart) / 1000;
    fill.style.width = `${90 * (1 - Math.exp(-t / 1.5))}%`;
  } else if (info?.doneAt && Date.now() - info.doneAt < DONE_MS) {
    fill.style.width = "100%";
  } else {
    fill.style.width = "0%";
  }
  requestAnimationFrame(drawProgress);
}
requestAnimationFrame(drawProgress);

// --- Diálogos (copiados de Garita) ----------------------------------------------------
//
// Las páginas (webviews hijos) flotan ENCIMA de esta ui: mientras un diálogo
// esté abierto, la pestaña activa se esconde (command shade) para no taparlo.

function openDialog(id) {
  call("shade", { on: true });
  $("overlay").classList.remove("hidden");
  // Cancela cualquier cierre en curso y oculta el resto de diálogos.
  document.querySelectorAll(".dlg").forEach((d) => {
    d.classList.add("hidden");
    d.classList.remove("closing");
  });
  $(id).classList.remove("hidden"); // al mostrarse, el CSS lo desliza hacia arriba
}

function closeDialogs() {
  const open = document.querySelector(".dlg:not(.hidden):not(.closing)");
  const done = () => {
    $("overlay").classList.add("hidden");
    call("shade", { on: false }); // la página vuelve cuando el velo ya no está
  };
  if (!open) return done();
  // Desliza el diálogo hacia abajo y oculta al terminar la animación.
  open.classList.add("closing");
  // Sin animación (p. ej. prefers-reduced-motion): ocultar directamente.
  if (getComputedStyle(open).animationName === "none") {
    open.classList.remove("closing");
    open.classList.add("hidden");
    return done();
  }
  open.addEventListener(
    "animationend",
    () => {
      // Si mientras tanto se abrió otro diálogo, openDialog ya limpió esto.
      if (!open.classList.contains("closing")) return;
      open.classList.remove("closing");
      open.classList.add("hidden");
      if (!document.querySelector(".dlg:not(.hidden)")) done();
    },
    { once: true }
  );
}

document.querySelectorAll("[data-close]").forEach((b) =>
  b.addEventListener("click", closeDialogs));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeDialogs();
  // Como en Garita: ⇧? enseña los atajos (salvo escribiendo en la barra).
  if (e.key === "?" && document.activeElement !== $("url")) openDialog("dlg-help");
});

// --- Tema (papel y tinta, copiado de Garita; se recuerda entre sesiones) --------------
//
// Todo el estilo cuelga de dos colores: --paper (fondo) y --ink (texto). Los
// tonos intermedios se derivan mezclándolos, y la clase .dark del body se
// decide por la luminosidad del papel. Además el tema viaja al backend
// (set_theme): de él salen la tinta de las páginas y la casa en blanco.

const THEME_PRESETS = {
  claro: { paper: "#f6f1e5", ink: "#23211b" },
  oscuro: { paper: "#201e19", ink: "#e6e0d0" },
};

/** Mezcla dos colores "#rrggbb": t=1 devuelve `a`, t=0 devuelve `b`. */
function mixHex(a, b, t) {
  const pa = a.match(/\w\w/g).map((x) => parseInt(x, 16));
  const pb = b.match(/\w\w/g).map((x) => parseInt(x, 16));
  return "#" + pa.map((v, i) =>
    Math.round(v * t + pb[i] * (1 - t)).toString(16).padStart(2, "0")).join("");
}

function isDarkColor(hex) {
  const [r, g, b] = hex.match(/\w\w/g).map((x) => parseInt(x, 16));
  return (r * 299 + g * 587 + b * 114) / 1000 < 128;
}

function applyTheme(theme) {
  const { paper, ink } = theme;
  const root = document.documentElement.style;
  root.setProperty("--paper", paper);
  root.setProperty("--ink", ink);
  root.setProperty("--ink-dim", mixHex(ink, paper, 0.5));
  root.setProperty("--ink-faint", mixHex(ink, paper, 0.2));
  document.body.classList.toggle("dark", isDarkColor(paper));
  localStorage.setItem("core-theme", JSON.stringify(theme));
  $("theme-paper").value = paper;
  $("theme-ink").value = ink;
  call("set_theme", { paper, ink }); // la tinta de las páginas sigue al tema
}

function loadTheme() {
  try {
    const saved = JSON.parse(localStorage.getItem("core-theme"));
    if (saved?.paper && saved?.ink) return saved;
  } catch { /* sin tema guardado */ }
  return THEME_PRESETS.claro;
}

function themeFromInputs() {
  return { paper: $("theme-paper").value, ink: $("theme-ink").value };
}

$("theme").addEventListener("click", () => openDialog("dlg-theme"));
$("theme-paper").addEventListener("input", () => applyTheme(themeFromInputs()));
$("theme-ink").addEventListener("input", () => applyTheme(themeFromInputs()));
$("theme-swap").addEventListener("click", () => {
  const t = themeFromInputs();
  applyTheme({ paper: t.ink, ink: t.paper });
});
$("theme-light").addEventListener("click", () => applyTheme(THEME_PRESETS.claro));
$("theme-dark").addEventListener("click", () => applyTheme(THEME_PRESETS.oscuro));

// --- Historial (páginas visitadas; se recuerda entre sesiones) ------------------------

const HISTORY_MAX = 500;

let visited = [];
try {
  visited = JSON.parse(localStorage.getItem("core-history")) ?? [];
} catch { /* sin historial guardado */ }

const saveVisited = () =>
  localStorage.setItem("core-history", JSON.stringify(visited));

/** Apunta una visita (la llama coreLoaded); recargar no duplica. */
function addVisit(url) {
  if (isBlank(url)) return;
  if (visited[0]?.url === url) {
    visited[0].when = Date.now();
  } else {
    visited.unshift({ url, title: null, when: Date.now() });
    if (visited.length > HISTORY_MAX) visited.length = HISTORY_MAX;
  }
  saveVisited();
}

/** Hora si es de hoy; fecha si no. */
function whenLabel(ts) {
  const d = new Date(ts);
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString();
}

function renderHistory() {
  const list = $("history-list");
  list.innerHTML = "";
  if (!visited.length) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "sin páginas visitadas";
    list.appendChild(li);
    return;
  }
  for (const v of visited) {
    const li = document.createElement("li");
    li.title = v.url;
    const title = document.createElement("span");
    title.className = "hist-title";
    title.textContent = v.title ?? tabLabel(v);
    const url = document.createElement("span");
    url.className = "hist-url dimmed";
    url.textContent = v.url;
    const when = document.createElement("span");
    when.className = "hist-when dimmed";
    when.textContent = whenLabel(v.when);
    li.append(title, url, when);
    li.addEventListener("click", () => {
      call("navigate", { url: v.url });
      closeDialogs();
    });
    list.appendChild(li);
  }
}

$("history").addEventListener("click", () => {
  renderHistory();
  openDialog("dlg-history");
});
$("history-clear").addEventListener("click", () => {
  visited = [];
  saveVisited();
  renderHistory();
});

// --- Marcadores (mitad baja de la columna derecha; se recuerdan entre sesiones) -------

let marks = [];
try {
  marks = JSON.parse(localStorage.getItem("core-marks")) ?? [];
} catch { /* sin marcadores guardados */ }

const saveMarks = () => localStorage.setItem("core-marks", JSON.stringify(marks));

function renderMarks() {
  const list = $("mark-list");
  list.innerHTML = "";
  if (!marks.length) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "sin marcadores";
    list.appendChild(li);
    return;
  }
  for (const [i, m] of marks.entries()) {
    const li = document.createElement("li");
    li.title = m.url;
    const title = document.createElement("span");
    title.className = "tab-title";
    title.textContent = tabLabel(m);
    const close = document.createElement("button");
    close.className = "tab-close";
    close.title = "quitar marcador";
    close.textContent = "✕";
    close.addEventListener("click", (e) => {
      e.stopPropagation(); // que no navegue de paso
      marks.splice(i, 1);
      saveMarks();
      renderMarks();
    });
    li.append(title, close);
    li.addEventListener("click", () => call("navigate", { url: m.url }));
    list.appendChild(li);
  }
}

/** Guarda la página de la pestaña activa (si no está ya guardada). */
$("mark-add").addEventListener("click", () => {
  const info = tabs.get(active);
  if (!info || isBlank(info.url)) return;
  const i = marks.findIndex((m) => m.url === info.url);
  if (i >= 0) {
    marks[i].title = info.title ?? marks[i].title; // ya estaba: refresca el título
  } else {
    marks.push({ url: info.url, title: info.title });
  }
  saveMarks();
  renderMarks();
});

renderMarks();

// --- Descargas (las de esta sesión; los ficheros van a ~/Descargas) -------------------

const dls = []; // {url, name, path, state: "en curso" | "ok" | "error", bytes, bps}

/** "1,3 MB", "840 KB"… (para el tamaño y la velocidad de descarga). */
function fmtSize(n) {
  if (!n) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${i && n < 10 ? n.toFixed(1) : Math.round(n)} ${units[i]}`;
}

function renderDownloads() {
  const list = $("downloads-list");
  list.innerHTML = "";
  if (!dls.length) {
    const li = document.createElement("li");
    li.className = "empty";
    li.textContent = "sin descargas en esta sesión";
    list.appendChild(li);
    return;
  }
  for (const d of dls) {
    const li = document.createElement("li");
    li.title = d.url;
    const name = document.createElement("span");
    name.className = "dl-name";
    name.textContent = d.name;
    li.append(name);
    if (d.state === "ok" && d.path) {
      const open = document.createElement("button");
      open.className = "btn";
      open.textContent = "abrir";
      open.addEventListener("click", () => call("open_path", { path: d.path, reveal: false }));
      const reveal = document.createElement("button");
      reveal.className = "btn";
      reveal.textContent = "carpeta";
      reveal.title = "enseñar en el Finder";
      reveal.addEventListener("click", () => call("open_path", { path: d.path, reveal: true }));
      li.append(open, reveal);
    } else {
      const state = document.createElement("span");
      state.className = "dl-state dimmed";
      state.textContent =
        d.state === "en curso"
          ? d.bytes ? `${fmtSize(d.bytes)} · ${fmtSize(d.bps)}/s` : "en curso…"
          : "falló";
      li.append(state);
    }
    list.appendChild(li);
  }
}

$("downloads").addEventListener("click", () => {
  renderDownloads();
  openDialog("dlg-downloads");
});
$("downloads-clear").addEventListener("click", () => {
  dls.length = 0;
  renderDownloads();
});

// El botón de info abre los atajos (igual que ⇧? o ⌘/).
$("help").addEventListener("click", () => openDialog("dlg-help"));

/** Empieza una descarga: se apunta (con su ruta de destino, que el Finished
 *  de macOS no la trae) y se abre el desplegable, que se vea. */
window.coreDownloadStarted = (url, name, path) => {
  dls.unshift({ url, name, path, state: "en curso", bytes: 0, bps: 0 });
  renderDownloads();
  openDialog("dlg-downloads");
};

/** Avance (lo mide el backend vigilando el fichero): tamaño y velocidad. */
window.coreDownloadProgress = (url, bytes, bps) => {
  const d = dls.find((d) => d.url === url && d.state === "en curso");
  if (!d) return;
  d.bytes = bytes;
  d.bps = bps;
  renderDownloads();
};

/** Terminó (bien o mal) la descarga más reciente de esa url. */
window.coreDownloadFinished = (url, path, success) => {
  const d = dls.find((d) => d.url === url && d.state === "en curso");
  if (!d) return;
  if (path) d.path = path; // si no viene (macOS), vale la apuntada al empezar
  d.state = success ? "ok" : "error";
  renderDownloads();
};

// --- Avisos del backend -----------------------------------------------------------

/** Pestaña nueva (creada por el backend); si isActive, pasa a ser la activa. */
window.coreTabOpened = (label, isActive) => {
  tabs.set(label, { url: null, title: null, loading: true });
  if (isActive) setActive(label);
  else renderTabs();
};

/** Empieza la carga de una pestaña. */
window.coreLoading = (label, url) => {
  const info = tabs.get(label);
  if (!info) return;
  // Una redirección en cadena no reinicia la barra: se conserva el arranque.
  if (!info.loadStart) info.loadStart = Date.now();
  info.loading = true;
  info.url = url;
  if (label === active) setActive(label);
  else renderTabs();
};

/** Carga terminada. */
window.coreLoaded = (label, url) => {
  const info = tabs.get(label);
  if (!info) return;
  info.loading = false;
  info.loadStart = null;
  info.doneAt = Date.now(); // la barra remata al 100% un instante
  info.url = url;
  addVisit(url); // al historial
  if (label === active) setActive(label);
  else renderTabs();
};

/** El backend activó una pestaña (p. ej. tras cerrar la que lo estaba). */
window.coreTabSelected = (label) => {
  if (tabs.has(label)) setActive(label);
};

/** Pestaña cerrada. */
window.coreTabClosed = (label) => {
  tabs.delete(label);
  if (active === label) active = null;
  renderTabs();
};

/** El menú de la app pide abrir un diálogo (los atajos ⌘H, ⌘J, ⌘,, ⌘/). */
window.coreMenu = (id) => {
  if (id === "dlg-history") renderHistory();
  if (id === "dlg-downloads") renderDownloads();
  openDialog(id);
};

/** El menú de la app pide el foco en la barra de dirección (⌘L). */
window.coreFocusUrl = () => $("url").focus();

/** La página cambió de título. */
window.coreTitle = (label, title) => {
  const info = tabs.get(label);
  if (!info) return;
  info.title = title;
  // El historial apunta la url al cargar, antes de conocer el título: se completa aquí.
  const v = visited.find((v) => v.url === info.url);
  if (v && title && v.title !== title) {
    v.title = title;
    saveVisited();
  }
  if (label === active) $("browser-title").textContent = `navegador · ${title}`;
  renderTabs();
};

// --- Splash de arranque (logo ASCII revelado de abajo arriba, como Garita) ------------

const SPLASH_ART = ` ░███████ ░██   ░██          ░███████
░██       ░██  ░██              ░██
 ░██████  ░█████     ░██████    ░██
      ░██ ░██  ░██              ░██
░███████  ░██   ░██          ░███████`;

let splashActive = true; // true mientras el splash sigue en pantalla

function dismissSplash() {
  const splash = $("splash");
  if (!splash || !splashActive) return;
  splashActive = false;
  splash.classList.add("done"); // fundido de salida (transition en CSS)
  splash.addEventListener("transitionend", () => splash.remove(), { once: true });
  setTimeout(() => splash.remove(), 700); // red de seguridad si no hay transición
  // La primera pestaña se pide AQUÍ y no antes: su webview flota encima de la
  // ui y taparía el logo. (Y nunca antes de cargar este script: los avisos
  // core* llegarían sin nadie que los escuchara y quedaría huérfana.)
  call("new_tab");
}

function runSplash() {
  // El sonido de arranque, discreto como en Garita; si el sistema lo veta, silencio.
  const sound = new Audio("assets/sounds/arranque.flac");
  sound.volume = 0.5;
  sound.play().catch(() => {});

  const pre = $("splash-art");
  const lines = SPLASH_ART.split("\n");
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const step = 90; // ms entre filas

  lines.forEach((text, i) => {
    const row = document.createElement("div");
    row.textContent = text || " ";
    if (!reduce) {
      row.classList.add("splash-line");
      // La fila de abajo se enciende primero; la de arriba, la última.
      row.style.animationDelay = `${(lines.length - 1 - i) * step}ms`;
    }
    pre.appendChild(row);
  });

  const total = reduce ? 400 : (lines.length - 1) * step + 250 + 400; // revelado + pausa
  setTimeout(dismissSplash, total);
}

// --- Arranque -----------------------------------------------------------------------

// El tema guardado se aplica ANTES del splash y de la primera pestaña, para
// que el logo, la tinta y el papel en blanco nazcan con los colores buenos.
applyTheme(loadTheme());
runSplash();
