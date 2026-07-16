// Core — navegador-lector mínimo con estética «papel y tinta».
//
// Una ventana con la interfaz propia (webview «ui», a pantalla completa:
// columna de navegación, barra de búsqueda, columna de pestañas y un bloque
// vacío enmarcado para el navegador) y un webview hijo por pestaña, colocado
// sobre el hueco del bloque navegador (API multiwebview, feature `unstable`).
// Solo la ui tiene acceso a la IPC; las pestañas son URLs remotas.
//
// El «modo tinta» es obligatorio: un initialization script inyecta la hoja
// papel-y-tinta en cada página ANTES del primer pintado (sin parpadeo) y se
// reafirma en cada fase de la carga.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, Webview, WebviewBuilder},
    window::WindowBuilder,
    AppHandle, LogicalPosition, LogicalSize, Manager, State, Url, WebviewUrl, Window,
};

/// Página de inicio de las pestañas nuevas: en blanco (papel vacío).
/// Va como data: con el color del papel del tema en el propio HTML — en
/// about:blank los initialization scripts (la tinta) no se ejecutan y quedaba
/// blanca. Las búsquedas desde la barra sí van a DuckDuckGo (to_url).
fn home_url(app: &AppHandle) -> Url {
    let paper = app.state::<TabsState>().0.lock().unwrap().theme.0.clone();
    format!(
        "data:text/html,%3Chtml%20style%3D%22background:{}%22%3E%3C/html%3E",
        paper.replace('#', "%23")
    )
    .parse()
    .unwrap()
}

// Geometría (px lógicos). El CSS de la ui dibuja los bloques con estos mismos
// números (core.css): si cambian aquí, deben cambiar allí.
const PAD_X: f64 = 12.0;   // margen exterior izquierdo/derecho
const PAD_TOP: f64 = 18.0; // margen superior (hueco para los títulos flotantes)
const PAD_BOT: f64 = 12.0; // margen inferior
const NAV_W: f64 = 44.0;   // columna de botones de navegación
const TABS_W: f64 = 180.0; // columna de pestañas
const BAR_H: f64 = 40.0;   // barra de búsqueda
const GAP: f64 = 12.0;     // separación entre bloques
const INSET: f64 = 8.0;    // respiro entre el borde del bloque navegador y la página

struct Tabs {
    order: Vec<String>,    // etiquetas de webview, en el orden de la columna
    active: Option<String>,
    counter: usize,        // para etiquetas únicas tab-1, tab-2…
    // Rectángulo del hueco del navegador, medido por la UI (getBoundingClientRect
    // del bloque + INSET) y comunicado con el command set_hole: así la página
    // queda clavada al marco dibujado por el CSS, sin duplicar geometría.
    hole: Option<(f64, f64, f64, f64)>,
    // Corrección de calibración (px lógicos) sumada a la posición del hueco:
    // compensa el desfase con que macOS coloca los webviews hijos.
    cal: (f64, f64),
    // Tema (papel, tinta) en "#rrggbb": lo manda la ui (set_theme) y de él
    // salen la hoja de tinta de las páginas y el color de la casa en blanco.
    theme: (String, String),
    // Descargas en curso (url → ruta de destino). Sirve de doble apunte: el
    // hilo que mide la velocidad para cuando su url desaparece de aquí.
    downloading: std::collections::HashMap<String, String>,
}

struct TabsState(Mutex<Tabs>);

/// Hoja de estilos del modo tinta: los dos colores del tema, tipografía mono,
/// imágenes en escala de grises. `!important` a todo: es una fotocopia.
///
/// Todo queda transparente sobre el papel del html — salvo lo que FLOTA
/// (desplegables, menús, diálogos): eso recibe papel opaco, que si no se
/// transparenta y el texto de debajo se cuela a través. Aquí van los patrones
/// reconocibles por selector; los demás los caza el vigilante del script
/// (INK_JS) mirando la posición computada.
fn ink_css(paper: &str, ink: &str) -> String {
    format!(
        "\
html {{ background: {paper} !important; }}\
*, *::before, *::after {{\
  background-color: transparent !important;\
  background-image: none !important;\
  color: {ink} !important;\
  font-family: ui-monospace, Menlo, Monaco, Consolas, 'DejaVu Sans Mono', monospace !important;\
  text-shadow: none !important;\
  box-shadow: none !important;\
  border-color: {ink} !important;\
  border-radius: 0 !important;\
}}\
body {{ background: {paper} !important; }}\
a, a * {{ color: {ink} !important; text-decoration: underline !important; }}\
img, video, canvas, svg, iframe {{ filter: grayscale(1) sepia(0.12) contrast(1.05) !important; }}\
select, option, optgroup, datalist, dialog, [popover],\
[role='dialog'], [role='alertdialog'], [role='menu'], [role='menubar'],\
[role='listbox'], [role='tooltip'], [role='combobox']\
{{ background-color: {paper} !important; }}\
"
    )
}

/// Plantilla del script de tinta (los __HUECOS__ los rellena ink_init_script;
/// así no hay que escapar llaves en un format!, como en el bloqueador).
///
/// Además de adoptar la hoja, un vigilante recorre el DOM y pinta de papel
/// opaco lo que flota sobre la página — que la hoja lo deja todo transparente
/// y un desplegable transparente es una sopa de letras:
///  - fixed y absolute con z-index alto (paneles: desplegables, diálogos,
///    drawers) → papel + filete de tinta, que se lea como panel;
///  - sticky (cabeceras que acompañan el scroll) → papel a secas.
/// También adopta la hoja en los shadow roots abiertos, donde no llega la
/// del documento. Corre al arrancar, al DOMContentLoaded y con un
/// MutationObserver (amortiguado a un frame) para lo que aparezca después.
const INK_JS: &str = "\
(function() {\
  var CSS = __CSS__;\
  window.__core_paper = __PAPER__;\
  window.__core_ink = __INK__;\
  function apply() {\
    try {\
      if (!window.__core_sheet) {\
        window.__core_sheet = new CSSStyleSheet();\
      }\
      window.__core_sheet.replaceSync(CSS);\
      if (!document.adoptedStyleSheets.includes(window.__core_sheet)) {\
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, window.__core_sheet];\
      }\
    } catch (e) {\
      if (!document.documentElement) return;\
      var s = document.getElementById('__core_ink');\
      if (!s) {\
        s = document.createElement('style');\
        s.id = '__core_ink';\
        document.documentElement.appendChild(s);\
      }\
      s.textContent = CSS;\
    }\
  }\
  function floats(el) {\
    var cs = getComputedStyle(el);\
    if (cs.pointerEvents === 'none' || cs.visibility === 'hidden') return null;\
    if (cs.position === 'fixed') return 'panel';\
    if (cs.position === 'sticky') return 'barra';\
    if (cs.position === 'absolute' && parseInt(cs.zIndex, 10) >= 10) return 'panel';\
    return null;\
  }\
  function fixup(root) {\
    if (!root.querySelectorAll) return;\
    root.querySelectorAll('*').forEach(function (el) {\
      if (el.shadowRoot) {\
        try {\
          if (window.__core_sheet && !el.shadowRoot.adoptedStyleSheets.includes(window.__core_sheet)) {\
            el.shadowRoot.adoptedStyleSheets = [...el.shadowRoot.adoptedStyleSheets, window.__core_sheet];\
          }\
        } catch (e) {}\
        fixup(el.shadowRoot);\
      }\
      var f = floats(el);\
      if (f) {\
        var want = f + window.__core_paper + window.__core_ink;\
        if (el.__core_float === want) return;\
        el.__core_float = want;\
        el.style.setProperty('background-color', window.__core_paper, 'important');\
        if (f === 'panel') {\
          el.style.setProperty('outline', '1px solid ' + window.__core_ink, 'important');\
        } else {\
          el.style.removeProperty('outline');\
        }\
      } else if (el.__core_float) {\
        el.__core_float = null;\
        el.style.removeProperty('background-color');\
        el.style.removeProperty('outline');\
      }\
    });\
  }\
  var queued = false;\
  function schedule() {\
    if (queued) return;\
    queued = true;\
    requestAnimationFrame(function () {\
      queued = false;\
      fixup(document);\
    });\
  }\
  apply();\
  fixup(document);\
  if (!window.__core_watch) {\
    window.__core_watch = true;\
    document.addEventListener('DOMContentLoaded', function () {\
      apply();\
      fixup(document);\
      new MutationObserver(schedule).observe(document.documentElement, {\
        childList: true, subtree: true, attributes: true, attributeFilter: ['style', 'class']\
      });\
    });\
  }\
})();";

/// Script de inicialización de cada pestaña: aplica la tinta en cuanto existe
/// el documento (antes del primer pintado) y la reafirma al cargar el DOM.
///
/// La hoja entra como *constructed stylesheet* (`adoptedStyleSheets`), no como
/// un `<style>` en el DOM: la CSP de algunas páginas (p. ej. DuckDuckGo, con
/// `style-src` estricto) bloquea los `<style>` inyectados, pero no alcanza a
/// las hojas construidas por CSSOM. Si no hay soporte, cae al `<style>`.
///
/// El script es idempotente: en las reafirmaciones (fases de carga, cambio de
/// tema con eval) reescribe la hoja y repinta los flotantes, pero el
/// MutationObserver solo se engancha una vez (window.__core_watch).
fn ink_init_script(app: &AppHandle) -> String {
    let (paper, ink) = app.state::<TabsState>().0.lock().unwrap().theme.clone();
    INK_JS
        .replace("__CSS__", &serde_json::to_string(&ink_css(&paper, &ink)).unwrap())
        .replace("__PAPER__", &serde_json::to_string(&paper).unwrap())
        .replace("__INK__", &serde_json::to_string(&ink).unwrap())
}

// --- Bloqueador de publicidad ---------------------------------------------------------
//
// Sin content rule lists (wry no expone las de WKWebView), el bloqueo va en
// dos capas: dentro de la página (CSS genérico que esconde los huecos de
// anuncios + retirada de iframes que cargan de dominios publicitarios) y en
// el backend (las «ventanas nuevas» hacia esos dominios se deniegan a secas:
// adiós popups y popunders).

/// Dominios de publicidad y sus redes habituales.
const AD_HOSTS: &[&str] = &[
    "doubleclick.net", "googlesyndication.com", "googleadservices.com",
    "googletagservices.com", "adnxs.com", "criteo.com", "criteo.net",
    "taboola.com", "outbrain.com", "amazon-adsystem.com", "rubiconproject.com",
    "pubmatic.com", "openx.net", "adform.net", "smartadserver.com", "teads.tv",
    "moatads.com", "2mdn.net", "adsafeprotected.com", "yieldmo.com",
    "sharethrough.com", "media.net", "adroll.com", "quantserve.com",
    "scorecardresearch.com", "zedo.com", "mgid.com", "revcontent.com",
    "popads.net", "propellerads.com",
];

/// Selectores genéricos de huecos de publicidad (recorte conservador de las
/// reglas cosméticas tipo EasyList: nada de clases ambiguas).
const AD_CSS: &str = "\
ins.adsbygoogle, .adsbygoogle, [id^='google_ads_'], [id^='div-gpt-ad'], \
iframe[id^='google_ads_iframe'], [id^='taboola-'], .trc_related_container, \
.OUTBRAIN, .advertisement, .advert, .ad-banner, .ad-container, .ad-wrapper, \
.ad-slot, .ad-unit, .ad-box, [aria-label='Advertisement' i] \
{ display: none !important; }";

/// Plantilla del script del bloqueador (los __HUECOS__ los rellena
/// adblock_init_script; así no hay que escapar llaves en un format!).
const ADBLOCK_JS: &str = "\
(function() {\
  var HOSTS = __HOSTS__;\
  var CSS = __CSS__;\
  function apply() {\
    try {\
      if (!window.__core_adsheet) {\
        window.__core_adsheet = new CSSStyleSheet();\
        window.__core_adsheet.replaceSync(CSS);\
      }\
      if (!document.adoptedStyleSheets.includes(window.__core_adsheet)) {\
        document.adoptedStyleSheets = [...document.adoptedStyleSheets, window.__core_adsheet];\
      }\
    } catch (e) {\
      if (!document.documentElement) return;\
      var s = document.getElementById('__core_adcss');\
      if (!s) {\
        s = document.createElement('style');\
        s.id = '__core_adcss';\
        document.documentElement.appendChild(s);\
      }\
      s.textContent = CSS;\
    }\
  }\
  function isAd(src) {\
    try {\
      var h = new URL(src, location.href).hostname;\
      return HOSTS.some(function (d) { return h === d || h.endsWith('.' + d); });\
    } catch (e) { return false; }\
  }\
  function sweep(root) {\
    if (!root.querySelectorAll) return;\
    root.querySelectorAll('iframe[src]').forEach(function (f) {\
      if (isAd(f.src)) f.remove();\
    });\
  }\
  apply();\
  document.addEventListener('DOMContentLoaded', function () {\
    apply();\
    sweep(document);\
    new MutationObserver(function (muts) {\
      muts.forEach(function (m) {\
        m.addedNodes.forEach(function (n) {\
          if (n.tagName === 'IFRAME' && n.src && isAd(n.src)) n.remove();\
          else sweep(n);\
        });\
      });\
    }).observe(document.documentElement, { childList: true, subtree: true });\
  });\
})();";

/// Script del bloqueador con la lista de dominios y el CSS ya incrustados.
fn adblock_init_script() -> String {
    ADBLOCK_JS
        .replace("__HOSTS__", &serde_json::to_string(AD_HOSTS).unwrap())
        .replace("__CSS__", &serde_json::to_string(AD_CSS).unwrap())
}

/// ¿Apunta la url a un dominio de publicidad?
fn is_ad_url(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|h| AD_HOSTS.iter().any(|d| h == *d || h.ends_with(&format!(".{d}"))))
}

/// Codificación percent mínima para la consulta de búsqueda.
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Convierte lo escrito en la barra en una URL: esquema explícito → tal cual;
/// algo con punto y sin espacios → https://; lo demás → búsqueda en DuckDuckGo.
fn to_url(input: &str) -> Result<Url, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err("dirección vacía".into());
    }
    let candidate = if t.contains("://") {
        t.to_string()
    } else if t.contains('.') && !t.contains(' ') {
        format!("https://{t}")
    } else {
        format!("https://duckduckgo.com/html/?q={}", urlencode(t))
    };
    candidate.parse::<Url>().map_err(|e| e.to_string())
}

/// Rectángulo (posición y tamaño lógicos) del hueco del navegador: el que
/// midió la UI si ya lo comunicó; si no, una estimación con las constantes.
///
/// La medida de la UI es relativa a su propio webview, que en macOS puede no
/// estar en el (0,0) de la ventana (la barra de título de por medio): se le
/// suma la posición real del webview de la UI para pasarla a coordenadas de
/// ventana, que son las que usa set_position en los webviews de pestaña.
fn hole(app: &AppHandle, window: &Window) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let s: LogicalSize<f64> = window
        .inner_size()
        .map(|p| p.to_logical(scale))
        .unwrap_or_else(|_| LogicalSize::new(1200.0, 840.0));
    let (x, y, w, h) = match app.state::<TabsState>().0.lock().unwrap().hole {
        Some(r) => r,
        None => {
            let x = PAD_X + NAV_W + GAP + INSET;
            let y = PAD_TOP + BAR_H + GAP + INSET;
            (
                x,
                y,
                (s.width - PAD_X - TABS_W - GAP - INSET - x).max(50.0),
                (s.height - PAD_BOT - INSET - y).max(50.0),
            )
        }
    };
    let cal = app.state::<TabsState>().0.lock().unwrap().cal;
    let _ = s;
    // El mismo desfase de 100px que en la posición: el webview pinta 100px
    // menos de alto de lo pedido, así que se piden 100 de más.
    (LogicalPosition::new(x + cal.0, y + cal.1), LogicalSize::new(w, h + 5.0))
}

/// Ejecuta JS en la interfaz (avisos de estado hacia la barra/pestañas).
fn ui_eval(app: &AppHandle, js: String) {
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.eval(js);
    }
}

fn active_webview(app: &AppHandle, tabs: &State<TabsState>) -> Result<Webview, String> {
    let label = tabs.0.lock().unwrap().active.clone().ok_or("sin pestaña activa")?;
    app.get_webview(&label).ok_or_else(|| "la pestaña activa no existe".into())
}

/// Ruta libre en `dir` para `name`: si ya existe, "nombre (1).ext", (2)…
fn unique_path(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let mut path = dir.join(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    let mut i = 1;
    while path.exists() {
        path = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    path
}

/// Crea una pestaña nueva (webview hijo sobre el hueco) y la activa.
fn spawn_tab(app: &AppHandle, url: Url) -> Result<String, String> {
    let window = app.get_window("main").ok_or("sin ventana")?;
    let tabs_state = app.state::<TabsState>();
    let label = {
        let mut t = tabs_state.0.lock().unwrap();
        t.counter += 1;
        format!("tab-{}", t.counter)
    };

    let load_handle = app.clone();
    let title_handle = app.clone();
    let dl_handle = app.clone();
    let nw_handle = app.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
        .initialization_script(ink_init_script(app))
        .initialization_script(adblock_init_script())
        // Enlaces target=_blank y window.open: sin este manejador la petición
        // de «ventana nueva» muere y el click parece roto. Aquí no hay
        // ventanas sueltas: se abre una pestaña nueva con esa url — salvo que
        // apunte a un dominio de publicidad (popups): esa se deniega a secas.
        .on_new_window(move |url, _features| {
            if !is_ad_url(&url) {
                // En un hilo aparte: crear el webview desde el hilo del event
                // loop cuelga WebView2 en Windows (ver nota en los commands).
                let h = nw_handle.clone();
                std::thread::spawn(move || {
                    let _ = spawn_tab(&h, url);
                });
            }
            NewWindowResponse::Deny
        })
        .on_page_load(move |webview, payload| {
            let label = serde_json::to_string(webview.label()).unwrap();
            let url = serde_json::to_string(payload.url().as_str()).unwrap();
            // Cinturón y tirantes: el init script ya tinta antes del primer
            // pintado, pero se reafirma en cada fase de la carga.
            match payload.event() {
                PageLoadEvent::Started => {
                    ui_eval(&load_handle, format!("coreLoading({label}, {url})"));
                    let _ = webview.eval(ink_init_script(&load_handle));
                }
                PageLoadEvent::Finished => {
                    ui_eval(&load_handle, format!("coreLoaded({label}, {url})"));
                    let _ = webview.eval(ink_init_script(&load_handle));
                }
            }
        })
        .on_document_title_changed(move |webview, title| {
            let label = serde_json::to_string(webview.label()).unwrap();
            let title = serde_json::to_string(&title).unwrap();
            ui_eval(&title_handle, format!("coreTitle({label}, {title})"));
        })
        // Descargas: a ~/Descargas con nombre único, avisando a la ui al
        // empezar y al terminar (el desplegable de descargas vive allí).
        .on_download(move |_webview, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    let name = destination
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .filter(|n| !n.is_empty())
                        .or_else(|| {
                            url.path_segments()
                                .and_then(|s| s.last())
                                .filter(|n| !n.is_empty())
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "descarga".into());
                    let dir = dl_handle
                        .path()
                        .download_dir()
                        .unwrap_or_else(|_| std::env::temp_dir());
                    *destination = unique_path(&dir, &name);
                    let file = destination.file_name().unwrap().to_string_lossy().into_owned();
                    let path = destination.to_string_lossy().into_owned();
                    let url_s = url.as_str().to_string();
                    // La ruta viaja YA en el aviso de inicio: el Finished de
                    // macOS llega siempre con path None (wry), aunque vaya bien.
                    ui_eval(&dl_handle, format!(
                        "coreDownloadStarted({}, {}, {})",
                        serde_json::to_string(&url_s).unwrap(),
                        serde_json::to_string(&file).unwrap(),
                        serde_json::to_string(&path).unwrap(),
                    ));
                    // Velocidad: wry no da progreso, pero WKDownload escribe
                    // directo al destino — un hilo mide el fichero cada medio
                    // segundo hasta que la url sale de `downloading`.
                    dl_handle.state::<TabsState>().0.lock().unwrap()
                        .downloading.insert(url_s.clone(), path.clone());
                    let th = dl_handle.clone();
                    std::thread::spawn(move || {
                        let mut last: u64 = 0;
                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(500));
                            let active = th.state::<TabsState>().0.lock().unwrap()
                                .downloading.contains_key(&url_s);
                            if !active {
                                break;
                            }
                            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            let bps = size.saturating_sub(last) * 2; // muestras de 500ms
                            last = size;
                            ui_eval(&th, format!(
                                "coreDownloadProgress({}, {size}, {bps})",
                                serde_json::to_string(&url_s).unwrap(),
                            ));
                        }
                    });
                }
                DownloadEvent::Finished { url, path, success } => {
                    // Se retira de las activas (el hilo de velocidad termina).
                    dl_handle.state::<TabsState>().0.lock().unwrap()
                        .downloading.remove(url.as_str());
                    let path = path.map(|p| p.to_string_lossy().into_owned());
                    ui_eval(&dl_handle, format!(
                        "coreDownloadFinished({}, {}, {success})",
                        serde_json::to_string(url.as_str()).unwrap(),
                        serde_json::to_string(&path).unwrap(),
                    ));
                }
                _ => {}
            }
            true // la descarga sigue adelante
        });

    let (pos, size) = hole(app, &window);
    window.add_child(builder, pos, size).map_err(|e| e.to_string())?;

    // Ocultar la anterior y activar esta.
    let mut t = tabs_state.0.lock().unwrap();
    if let Some(prev) = &t.active {
        if let Some(w) = app.get_webview(prev) {
            let _ = w.hide();
        }
    }
    t.order.push(label.clone());
    t.active = Some(label.clone());
    drop(t);
    ui_eval(app, format!("coreTabOpened({}, true)", serde_json::to_string(&label).unwrap()));
    Ok(label)
}

/// Recoloca la pestaña activa al hueco actual.
fn layout(app: &AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    let active = app.state::<TabsState>().0.lock().unwrap().active.clone();
    if let Some(label) = active {
        if let Some(w) = app.get_webview(&label) {
            let (pos, size) = hole(app, &window);
            let _ = w.set_position(pos);
            let _ = w.set_size(size);
        }
    }
}

/// La UI comunica el rectángulo medido del hueco (px lógicos, ya con el
/// respiro descontado); recolocamos la pestaña activa al momento.
#[tauri::command]
async fn set_hole(app: AppHandle, tabs: State<'_, TabsState>, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    println!("[core] hueco medido por la ui: x={x:.0} y={y:.0} {w:.0}×{h:.0}");
    if let Some(u) = app.get_webview("ui") {
        if let (Ok(p), Ok(s)) = (u.position(), u.size()) {
            println!("[core] ui en física: {},{} {}×{}", p.x, p.y, s.width, s.height);
        }
    }
    tabs.0.lock().unwrap().hole = Some((x, y, w.max(50.0), h.max(50.0)));
    layout(&app);
    // Traza: dónde quedó de verdad la pestaña activa (px físicos).
    let active = app.state::<TabsState>().0.lock().unwrap().active.clone();
    if let Some(w) = active.and_then(|l| app.get_webview(&l)) {
        if let (Ok(p), Ok(s)) = (w.position(), w.size()) {
            println!("[core] pestaña activa en física: {},{} {}×{}", p.x, p.y, s.width, s.height);
        }
    }
    Ok(())
}

// --- Commands (los invoca la ui) ---------------------------------------------

// IMPORTANTE (Windows): los métodos de ventana/webview (crear, navegar, mover,
// mostrar…) se cuelgan si se llaman en el hilo del event loop. Los commands
// van por eso `async` — Tauri los ejecuta fuera del hilo principal, y desde
// ahí `add_child` y compañía funcionan. En macOS/Linux es indiferente. Por lo
// mismo, `select_tab`/`close_tab` tienen una versión `_impl` síncrona: así se
// pueden encadenar entre sí (y llamarse desde un hilo en los manejadores de
// eventos) sin volver a pasar por la capa de command.

#[tauri::command]
async fn navigate(app: AppHandle, tabs: State<'_, TabsState>, url: String) -> Result<(), String> {
    let url = to_url(&url)?;
    active_webview(&app, &tabs)?.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
async fn nav_back(app: AppHandle, tabs: State<'_, TabsState>) -> Result<(), String> {
    active_webview(&app, &tabs)?.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
async fn nav_forward(app: AppHandle, tabs: State<'_, TabsState>) -> Result<(), String> {
    active_webview(&app, &tabs)?.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
async fn nav_reload(app: AppHandle, tabs: State<'_, TabsState>) -> Result<(), String> {
    active_webview(&app, &tabs)?.reload().map_err(|e| e.to_string())
}

#[tauri::command]
async fn new_tab(app: AppHandle) -> Result<String, String> {
    let home = home_url(&app);
    spawn_tab(&app, home)
}

/// Activa `label`: esconde la anterior, coloca esta en el hueco y la muestra.
fn select_tab_impl(app: &AppHandle, label: String) -> Result<(), String> {
    let window = app.get_window("main").ok_or("sin ventana")?;
    let tabs = app.state::<TabsState>();
    let mut t = tabs.0.lock().unwrap();
    if !t.order.contains(&label) {
        return Err("pestaña desconocida".into());
    }
    if let Some(prev) = &t.active {
        if prev != &label {
            if let Some(w) = app.get_webview(prev) {
                let _ = w.hide();
            }
        }
    }
    t.active = Some(label.clone());
    drop(t);
    if let Some(w) = app.get_webview(&label) {
        let (pos, size) = hole(app, &window);
        let _ = w.set_position(pos);
        let _ = w.set_size(size);
        let _ = w.show();
    }
    ui_eval(app, format!("coreTabSelected({})", serde_json::to_string(&label).unwrap()));
    Ok(())
}

#[tauri::command]
async fn select_tab(app: AppHandle, label: String) -> Result<(), String> {
    select_tab_impl(&app, label)
}

/// Cierra `label`; si era la activa, pasa a la vecina (o abre casa si no queda).
fn close_tab_impl(app: &AppHandle, label: String) -> Result<(), String> {
    if let Some(w) = app.get_webview(&label) {
        let _ = w.close();
    }
    let next = {
        let tabs = app.state::<TabsState>();
        let mut t = tabs.0.lock().unwrap();
        let Some(i) = t.order.iter().position(|l| l == &label) else {
            return Err("pestaña desconocida".into());
        };
        t.order.remove(i);
        if t.active.as_deref() == Some(label.as_str()) {
            t.active = t.order.get(i.saturating_sub(1)).or(t.order.first()).cloned();
            t.active.clone()
        } else {
            None // la activa no cambia: no hay que recolocar nada
        }
    };
    ui_eval(app, format!("coreTabClosed({})", serde_json::to_string(&label).unwrap()));
    if let Some(next) = next {
        select_tab_impl(app, next)?;
    } else if app.state::<TabsState>().0.lock().unwrap().order.is_empty() {
        // Sin pestañas no hay navegador: abrir una casa nueva.
        let home = home_url(app);
        spawn_tab(app, home)?;
    }
    Ok(())
}

#[tauri::command]
async fn close_tab(app: AppHandle, label: String) -> Result<(), String> {
    close_tab_impl(&app, label)
}

/// Lleva la pestaña activa a la página inicial (papel en blanco).
#[tauri::command]
async fn nav_home(app: AppHandle, tabs: State<'_, TabsState>) -> Result<(), String> {
    let home = home_url(&app);
    active_webview(&app, &tabs)?.navigate(home).map_err(|e| e.to_string())
}

/// Un "#rrggbb" de verdad: lo único que se acepta como color del tema.
fn valid_hex(c: &str) -> bool {
    c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// La ui manda el tema (papel y tinta): se guarda y se re-tintan todas las
/// pestañas al momento (el init script reescribe la hoja con replaceSync).
#[tauri::command]
async fn set_theme(app: AppHandle, tabs: State<'_, TabsState>, paper: String, ink: String) -> Result<(), String> {
    if !valid_hex(&paper) || !valid_hex(&ink) {
        return Err("color inválido: se espera #rrggbb".into());
    }
    let order = {
        let mut t = tabs.0.lock().unwrap();
        t.theme = (paper, ink);
        t.order.clone()
    };
    for label in order {
        if let Some(w) = app.get_webview(&label) {
            let _ = w.eval(ink_init_script(&app));
        }
    }
    Ok(())
}

/// Abre un fichero descargado (con `reveal`, lo enseña en el gestor de
/// archivos del sistema: Finder, Explorador o el que toque).
#[tauri::command]
fn open_path(path: String, reveal: bool) {
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if reveal {
            cmd.arg("-R");
        }
        let _ = cmd.arg(path).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = if reveal {
            std::process::Command::new("explorer").arg(format!("/select,{path}")).spawn()
        } else {
            std::process::Command::new("explorer").arg(path).spawn()
        };
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // xdg-open no sabe seleccionar: para revelar se abre la carpeta.
        let target = if reveal {
            std::path::Path::new(&path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(path)
        } else {
            path
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
}

/// Esconde (o repone) la pestaña activa. Los diálogos de la ui viven en un
/// webview que queda POR DEBAJO de las páginas: mientras haya uno abierto,
/// la página se aparta para no taparlo.
#[tauri::command]
async fn shade(app: AppHandle, tabs: State<'_, TabsState>, on: bool) -> Result<(), String> {
    let active = tabs.0.lock().unwrap().active.clone();
    if let Some(w) = active.and_then(|l| app.get_webview(&l)) {
        let _ = if on { w.hide() } else { w.show() };
    }
    Ok(())
}

fn main() {
    // WebView2 (Windows) bloquea el autoplay por defecto: sin esto, el sonido
    // de arranque quedaría mudo. Debe fijarse antes de crear ningún webview.
    #[cfg(target_os = "windows")]
    std::env::set_var(
        "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
        "--autoplay-policy=no-user-gesture-required",
    );

    tauri::Builder::default()
        .manage(TabsState(Mutex::new(Tabs {
            order: Vec::new(),
            active: None,
            counter: 0,
            hole: None,
            // macOS coloca los webviews hijos más arriba de lo pedido: se
            // compensa bajándolos (valor afinado a ojo). En Windows/Linux no
            // se ha observado ese desfase: sin corrección hasta calibrarlos.
            #[cfg(target_os = "macos")]
            cal: (0.0, 30.0),
            #[cfg(not(target_os = "macos"))]
            cal: (0.0, 0.0),
            theme: ("#f6f1e5".into(), "#23211b".into()),
            downloading: std::collections::HashMap::new(),
        })))
        .invoke_handler(tauri::generate_handler![
            navigate, nav_back, nav_forward, nav_reload, nav_home,
            new_tab, select_tab, close_tab, set_hole, set_theme, shade,
            open_path,
        ])
        .setup(|app| {
            let width = 1200.0;
            let height = 840.0;
            let window = WindowBuilder::new(app, "main")
                .title("Core")
                .inner_size(width, height)
                .min_inner_size(720.0, 480.0)
                .build()?;

            // La interfaz: pantalla completa, con el hueco del navegador vacío.
            let ui = WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()));
            window.add_child(
                ui,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(width, height),
            )?;

            // Menú de la app con los atajos de navegador. Van por menú y no
            // por keydown de la ui: los aceleradores funcionan aunque el foco
            // esté dentro de la página (donde la ui no ve el teclado).
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
                let h = app.handle();
                let item = |id: &str, label: &str, accel: &str| {
                    MenuItemBuilder::with_id(id, label).accelerator(accel).build(h)
                };
                let core = SubmenuBuilder::new(h, "Core")
                    .item(&PredefinedMenuItem::quit(h, None)?)
                    .build()?;
                let edicion = SubmenuBuilder::new(h, "Edición")
                    .undo().redo().separator().cut().copy().paste().select_all()
                    .build()?;
                let navegar = SubmenuBuilder::new(h, "Navegar")
                    .item(&item("back", "atrás", "CmdOrCtrl+BracketLeft")?)
                    .item(&item("forward", "adelante", "CmdOrCtrl+BracketRight")?)
                    .item(&item("reload", "recargar", "CmdOrCtrl+R")?)
                    .item(&item("home", "página inicial", "Shift+CmdOrCtrl+H")?)
                    .item(&item("url", "escribir dirección", "CmdOrCtrl+L")?)
                    .build()?;
                let pestanas = SubmenuBuilder::new(h, "Pestañas")
                    .item(&item("tab-new", "nueva pestaña", "CmdOrCtrl+T")?)
                    .item(&item("tab-close", "cerrar pestaña", "CmdOrCtrl+W")?)
                    .build()?;
                let ver = SubmenuBuilder::new(h, "Ver")
                    .item(&item("dlg-history", "historial", "CmdOrCtrl+H")?)
                    .item(&item("dlg-downloads", "descargas", "CmdOrCtrl+J")?)
                    .item(&item("dlg-theme", "tema", "CmdOrCtrl+Comma")?)
                    .item(&item("dlg-help", "atajos", "CmdOrCtrl+Slash")?)
                    .build()?;
                let menu = MenuBuilder::new(h)
                    .item(&core).item(&edicion).item(&navegar).item(&pestanas).item(&ver)
                    .build()?;
                app.set_menu(menu)?;
                app.on_menu_event(|app, event| {
                    let id = event.id().as_ref().to_string();
                    let app = app.clone();
                    // En un hilo aparte: navegar/crear/cerrar/enfocar desde el
                    // hilo del event loop cuelga WebView2 en Windows (misma nota
                    // que en los commands async). En mac/Linux es indiferente.
                    std::thread::spawn(move || {
                        // Los diálogos viven en la ui: abrirlo y darle el foco
                        // (para que esc y los clicks caigan allí, no en la página).
                        if id.starts_with("dlg-") {
                            if let Some(ui) = app.get_webview("ui") {
                                let _ = ui.set_focus();
                                let _ = ui.eval(format!(
                                    "coreMenu({})",
                                    serde_json::to_string(&id).unwrap()
                                ));
                            }
                            return;
                        }
                        let tabs = app.state::<TabsState>();
                        match id.as_str() {
                            "back" => {
                                if let Ok(w) = active_webview(&app, &tabs) {
                                    let _ = w.eval("history.back()");
                                }
                            }
                            "forward" => {
                                if let Ok(w) = active_webview(&app, &tabs) {
                                    let _ = w.eval("history.forward()");
                                }
                            }
                            "reload" => {
                                if let Ok(w) = active_webview(&app, &tabs) {
                                    let _ = w.reload();
                                }
                            }
                            "home" => {
                                let home = home_url(&app);
                                if let Ok(w) = active_webview(&app, &tabs) {
                                    let _ = w.navigate(home);
                                }
                            }
                            "url" => {
                                if let Some(ui) = app.get_webview("ui") {
                                    let _ = ui.set_focus();
                                    let _ = ui.eval("coreFocusUrl()");
                                }
                            }
                            "tab-new" => {
                                let home = home_url(&app);
                                let _ = spawn_tab(&app, home);
                            }
                            "tab-close" => {
                                let label = tabs.0.lock().unwrap().active.clone();
                                if let Some(label) = label {
                                    let _ = close_tab_impl(&app, label);
                                }
                            }
                            _ => {}
                        }
                    });
                });
            }

            // La primera pestaña no se abre aquí: la pide la ui al terminar de
            // cargar (app.js), cuando ya existen los core* que la registran.

            // Redimensionar: la ui sigue a la ventana; la pestaña activa, al hueco.
            let handle = app.handle().clone();
            let win = window.clone();
            window.on_window_event(move |event| {
                // ScaleFactorChanged cuenta también: al cambiar de monitor o
                // de escala (DPI fraccionario en Windows/Linux) hay que
                // recolocar igual que en un resize.
                if matches!(
                    event,
                    tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. }
                ) {
                    // En un hilo aparte: redimensionar los webviews desde el hilo
                    // del event loop cuelga WebView2 en Windows (misma nota que
                    // en los commands async). En mac/Linux es indiferente.
                    let handle = handle.clone();
                    let win = win.clone();
                    std::thread::spawn(move || {
                        if let Some(ui) = handle.get_webview("ui") {
                            let scale = win.scale_factor().unwrap_or(1.0);
                            if let Ok(size) = win.inner_size() {
                                let s: LogicalSize<f64> = size.to_logical(scale);
                                let _ = ui.set_size(s);
                            }
                        }
                        layout(&handle);
                    });
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al arrancar Core");
}
