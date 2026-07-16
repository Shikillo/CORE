# Core

Navegador-lector mínimo con estética «papel y tinta» (la de
[Garita](https://github.com/Shikillo/Garita)), construido con **Tauri 2** y su
API multiwebview.

## Estructura

```
┌──────┬──────────────────────────────┬────────────┐
│ nav  │  dirección              · │  pestañas  │
│      ├──────────────────────────────┤  tab 1     │
│ ◀ ▶  │                              │  tab 2     │
│ ↺ ⌂  │         navegador            │  + nueva   │
│ ◐    │                              ├────────────┤
│ ⧗ ↓  │                              │  marcadores│
│ ?    │                              │  + guardar │
└──────┴──────────────────────────────┴────────────┘
```

La interfaz es un webview propio a pantalla completa (HTML/CSS/JS vanilla,
sin Node ni bundler) con los bloques de borde y título flotante de Garita;
cada pestaña es **otro webview hijo** colocado sobre el hueco del bloque
navegador. Solo la interfaz tiene acceso a la IPC de Tauri: las pestañas son
URLs remotas sin acceso al backend.

## Qué hace

- **Modo tinta, de serie** — cada página se re-tinta a papel y tinta **antes
  del primer pintado** (initialization script): fondo papel, texto en tinta,
  tipografía mono, imágenes en escala de grises. El botón `◐` de la columna
  izquierda lo apaga/enciende para todas las pestañas.
- **Tema a tu gusto** — el botón `tema` abre un diálogo con dos colores,
  papel y tinta, un `⇄` para intercambiarlos y los presets `claro`/`oscuro`.
  El tema pinta la interfaz **y** la tinta de las páginas, y se recuerda
  entre sesiones.
- **Pestañas** — mitad alta de la columna derecha: título de la página (o
  dominio), `✕` para cerrar, «+ nueva» para abrir. Cerrar la última abre una
  pestaña de inicio.
- **Marcadores** — mitad baja de la columna derecha: «+ guardar» apunta la
  página actual, `✕` la quita; se recuerdan entre sesiones.
- **Dirección** — URL (con `https://` implícito) o texto, que se busca en
  DuckDuckGo. `Enter` navega, `Escape` suelta el foco; un `·` parpadea
  mientras carga la pestaña activa.
- **Navegación** — `◀ ▶ ↺ ⌂` (atrás, adelante, recargar, página inicial) en
  la columna izquierda, más historial (`⧗`) y descargas (`↓`).
- **Atajos** — el botón `?` (o `⌘/`) los lista: `⌘T` pestaña nueva, `⌘W`
  cerrar, `⌘R` recargar, `⌘[ ⌘]` atrás/adelante, `⇧⌘H` inicio, `⌘L` escribir
  dirección, `⌘H` historial, `⌘J` descargas, `⌘,` tema.

## Arrancar

```sh
cd src-tauri
cargo tauri dev
```

(Requiere `tauri-cli` 2.x: `cargo install tauri-cli`.)

## Arquitectura

- `src-tauri/src/main.rs` — todo el backend: ventana + webview de interfaz +
  un webview por pestaña (`window.add_child`, feature `unstable`), commands
  (`navigate`, `nav_back/forward/reload`, `new_tab`, `select_tab`,
  `close_tab`, `nav_home`, `set_theme`, `shade`, `set_hole`, `open_path`) y
  avisos hacia la interfaz por `eval` (`planetTabOpened/Selected/Closed`,
  `planetLoading/Loaded`, `planetTitle`). La geometría del hueco del
  navegador (`hole()`) está **calcada** en el grid de `core.css`: si cambia
  una, debe cambiar la otra.
- `src/` — la interfaz: `index.html`, `core.css`, `app.js` y `assets/`
  (iconos SVG y el sonido de arranque).

## Créditos

- **Shikillo** ([@Shikillo](https://github.com/Shikillo)) — autoría y diseño.
- Desarrollado con la ayuda de [Claude Code](https://claude.com/claude-code).
