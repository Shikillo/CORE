# Planet

Navegador-lector mínimo con estética «papel y tinta» (la de
[Garita](https://github.com/Shikillo/Garita)), construido con **Tauri 2** y su
API multiwebview.

## Estructura

```
┌──────┬──────────────────────────────┬────────────┐
│ nav  │  dirección                   │  pestañas  │
│      ├──────────────────────────────┤            │
│ ◀    │                              │  tab 1     │
│ ▶    │                              │  tab 2     │
│ ↺    │         navegador            │            │
│      │                              │            │
│ ◐    │                              │  + nueva   │
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
- **Pestañas** — columna derecha: título de la página (o dominio), `✕` para
  cerrar, «+ nueva» para abrir. Cerrar la última abre una pestaña de inicio.
- **Dirección** — URL (con `https://` implícito) o texto, que se busca en
  DuckDuckGo. `Enter` navega, `Escape` suelta el foco; un `·` parpadea
  mientras carga la pestaña activa.
- **Navegación** — `◀ ▶ ↺` en la columna izquierda.

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
  `close_tab`, `set_ink`) y avisos hacia la interfaz por `eval`
  (`planetTabOpened/Selected/Closed`, `planetLoading/Loaded`, `planetTitle`).
  La geometría del hueco del navegador (`hole()`) está **calcada** en el grid
  de `planet.css`: si cambia una, debe cambiar la otra.
- `src/` — la interfaz: `index.html`, `planet.css`, `app.js`.

## Limitaciones conocidas (v0.1)

- La API multiwebview de Tauri está marcada como *unstable*.
- Los popups y `target="_blank"` no abren pestaña nueva (se ignoran).
- Sin historial persistente, marcadores ni descargas.
- El modo tinta es una fotocopia a la fuerza: en webs de texto queda
  precioso; en apps web complejas puede romper la maquetación. Apagarlo
  (`◐`) muestra la web original (con un parpadeo re-tintado al cargar).
- Atrás/adelante usan `history.back()/forward()` del propio webview.
