# Zipp Browser

A browser engine that runs page JavaScript on [ZIPP](https://github.com/f2i-com/zipp.org) (`zipp-vm 0.0.39`) —
a clean-sheet JS/TS engine with a register VM, native x86-64/ARM64 JITs, and Wasmtime-backed WebAssembly.

Everything below the JS is real engine code, not a toy: **Stylo** (Firefox's
style engine) for the CSS cascade, **Taffy** for flexbox/grid/block layout,
**Parley** + **Skrifa** for text shaping, and **tiny-skia** for fast cached software
rasterization, with **WebGL 1/2** and **WebGPU** for hardware-accelerated 3D canvases.
No GPU is required for page rendering — it runs over RDP, on bare-metal Linux KMS, and
headlessly inside mobile apps such as SoftN Mobile for Android.

```
fetch → zipp-vm (0.0.39 JIT) runs JS/TS/Wasm → Stylo cascade → Taffy layout → tiny-skia paint → winit / KMS / Android surface
```

## Status

Zipp Browser renders modern React/Tailwind/Three.js web apps and SoftN v2 apps (including
Shadow DOM + PHP/SQLite Wasm container apps, 2D tilemaps, and Three.js `.glb`/`.hdr` PBR
scenes) with live event handlers, WebGL 1 & 2, WebGPU, Canvas 2D, WebAssembly, Web Workers,
IndexedDB, cookies, PDF viewing, and both **Desktop** and **Mobile View** modes.

<!-- A screenshot lives at docs/screenshot.png once you generate one with --screenshot. -->

## Layout

| Crate | What it does |
| --- | --- |
| `crates/zipp-dom` | DOM, HTML parsing (`html5ever`), Stylo↔Taffy bridge, embedded Liberation fonts (Parley + `usvg`), layout, events |
| `crates/zipp-renderer` | Paint tree and cached `tiny-skia` rasterizer (box-shadow blur cache, glyph alpha cache, clip-mask pooling, partial canvas damage repaint) |
| `crates/zipp-shell` | `winit` windowing, networking, cookie jar, profile storage, live viewport resizing |
| `crates/zipp-kms` | Bare-metal Linux DRM/KMS + `evdev` display/input backend for `--kiosk` mode |
| `apps/browser` | The `zipp-browser` desktop & kiosk binary: `zipp-vm` integration, DOM/Canvas2D/WebGL/WebGPU/Worker shims, toolbar + Mobile View, console, PDF/media viewers |

`zipp-vm` (`0.0.39`) comes from [ZIPP](https://github.com/f2i-com/zipp.org) as a git
dependency pinned in `Cargo.lock` and consumed through its `embed` module — a persistent
VM compiled once, kept alive, and re-entered to dispatch DOM events, timers, and animation
frames into the same global context.

## Building

```bash
git clone git@github.com:f2i-com/zipp-browser.git
cd zipp-browser
cargo build --release
```

Cargo fetches `zipp-vm` automatically at the exact commit pinned in `Cargo.lock`; run
`cargo update -p zipp-vm` to move to a newer revision.

WebAssembly is compiled to machine code by Wasmtime (`zipp-vm`'s `wasm-jit` feature),
which adds a few minutes to a clean release build and ~9 MB to the binary.

### Building against a local `zipp.org` checkout

Keep `zipp-browser` and `zipp.org` side by side and add a `.cargo/config.toml` in their
shared parent directory:

```toml
[patch."https://github.com/f2i-com/zipp.org"]
zipp-vm = { path = "zipp.org/crates/zipp-vm" }
```

### Headless & Mobile Embedding (Android / SoftN Mobile)

`zipp-dom` and `zipp-renderer` can be embedded without `winit` or `softbuffer` (as used by
`softn-android` on Android) by disabling default windowing/system-font features:

```toml
zipp-dom = { path = "...", default-features = false, features = ["svg", "woff-rust", "block", "flexbox", "grid"] }
zipp-renderer = { path = "...", default-features = false, features = ["svg"] }
```

All 12 Liberation TTF faces are compiled directly into `zipp-dom` and registered into both
Parley and `usvg`'s `FONT_DB`, so both HTML text and inline SVG `<text>` render consistently
without system font directories.

## Running

```bash
# Desktop browser (1280x800 default viewport)
zipp-browser                                       # opens https://lance.name
zipp-browser https://example.com                   # opens a URL (bare domains like example.com work too)

# Mobile View (matches SoftN Mobile Android Pixel 9a logical viewport: 412x915)
zipp-browser --mobile https://example.com          # mobile viewport + Android mobile UA + touch hints
zipp-browser --viewport 390x844 https://example.com

# Chromeless App Mode (desktop or mobile preview for web / .softn apps)
zipp-browser --app https://example.com
zipp-browser --mobile --app path/to/index.html path/to/app.softn

# Bare-metal Linux DRM/KMS Kiosk Mode (Linux only)
zipp-browser --kiosk https://example.com

# Headless Screenshots (desktop, mobile, or chromeless mobile app)
zipp-browser --screenshot out.png https://example.com [width] [height]
zipp-browser --mobile --screenshot mobile.png https://example.com
zipp-browser --mobile --app --screenshot mobile-app.png path/to/index.html

# Built-in Test Suites & Info
zipp-browser --test-js                             # DOM/browser JS test suite
zipp-browser --test-compat                         # JS language compatibility suite
zipp-browser --test-vm                             # quick zipp-vm check
zipp-browser --help
zipp-browser --version
```

### Desktop Controls & Mobile View Toggle

- **Desktop ↔ Mobile View toggle:** Click the smartphone icon in the toolbar (next to the
  Console button) or press **`Ctrl+Shift+M`** to switch live between **Desktop (`1280×800`)**
  and **Mobile View (`412×915`)** — the same logical viewport used by SoftN Mobile on Android
  (`1080×2424 @ 2.625×`). Toggling resizes the window, updates Stylo `@media` queries,
  updates `window.innerWidth`/`innerHeight`, `screen.orientation`, `matchMedia`, and
  `navigator.maxTouchPoints`, and fires the DOM `resize` event.
- **Navigation:** Toolbar Back / Forward / Reload buttons, `Alt+Left` / `Alt+Right`,
  mouse back/forward buttons, `F5` / `Ctrl+R`, and `Ctrl+L` / `F6` for the address bar.
- **Scrolling & Zoom:** Arrow keys, Page Up/Down, Space, Home/End, mouse wheel, and
  `Ctrl+=` / `Ctrl+-` / `Ctrl+0`.
- **Developer Console:** `F12` or the toolbar console button (`>_`) opens the live console
  with error badge counts, `console.*` inspection, worker output, and an interactive JS REPL.
- **Layout Inspector:** `Alt+D` toggles layout boxes, `Alt+H` highlights hovered nodes,
  and `Alt+T` prints the Taffy layout tree.

### Headless Screenshot Options

`--screenshot` writes the post-JavaScript HTML alongside the PNG. Build with `--release`
so the JS engine and rasterizer run optimized.

Environment switches for automated visual tests:

- `ZIPP_UICLICK=x,y[;x,y…]` — dispatches real hit-tested clicks at client coordinates before capture.
- `ZIPP_RESIZE=w,h[;w,h…]` — resizes the live page through each size in turn.
- `ZIPP_COLOR_SCHEME=light|dark` — overrides OS `prefers-color-scheme` for both CSS and `matchMedia`.
- `ZIPP_EMBEDDED_FONTS_ONLY=1` — forces embedded Liberation fonts only for deterministic cross-machine renders.
- `ZIPP_DEBUG_CONSOLE=1` — mirrors page/worker `console.*` messages and errors to `stderr`.

## Rendering & Performance

`zipp-renderer`'s `TinySkiaPainter` is tuned for interactive UIs and 60 FPS 3D/canvas apps:

- **Downscaled Box-Shadow Blur Cache:** Large Gaussian shadows (`sigma >= 3px` / `>= 10px`)
  are blurred at 1/2× or 1/4× resolution and cached position-independently across scrolls
  and repaints.
- **Glyph Alpha-Mask Cache:** Non-variable axis-aligned glyphs are rasterized once into
  tight 1-byte coverage masks and composited directly.
- **Clip-Mask Pooling:** Full-framebuffer rounded clip masks are pooled and recycled by
  dirty row span instead of reallocating megabytes per layer.
- **Partial Canvas Overlay Repaint (`paint_scene_from_nodes`):** Embedded hosts (such as
  SoftN Mobile) can repaint only updated opaque `<canvas>` regions and their overlays while
  preserving static page pixels outside the canvas, cutting 3D animation paint times by >30×.
- **Fast Fills & Image Blits:** Solid axis-aligned rects bypass path tessellation, gradient
  fills clamp to the visible clip/viewport, and `draw_image` uses a precomputed row LUT with
  opaque direct-copy fast paths.

## Video

On Windows, `<video>` plays through Media Foundation (H.264, VP9, AV1, MPEG-4 in MP4, WebM,
QuickTime) with sound and network streaming. Frames copy out at display size without
restyling or re-laying out the page. Click to play/pause; `controls` adds seek/mute UI.
`ZIPP_MUTE_MEDIA=1` mutes playback for automated runs; `ZIPP_DEBUG_MEDIA=1` logs media events.
On other platforms and in `--screenshot`, `<video>` renders its poster image.

## WebAssembly, Workers, Canvas 2D, WebGL and WebGPU

- **WebAssembly:** Full JS `WebAssembly` API compiled to native machine code via Wasmtime's
  Cranelift (`wasm-jit`), including `instantiateStreaming`, `wasm-bindgen`, and `externref`.
- **Web Workers:** `new Worker(url)` (classic or `{ type: "module" }`) runs in its own VM
  thread (`worker.rs` + `worker_shim.js`) with `fetch`, `URL`, timers, `importScripts`,
  `WebAssembly`, and structured-clone messaging.
- **Canvas 2D:** `getContext("2d")` draws via `tiny-skia` (`canvas2d.rs` + `canvas2d_shim.js`)
  with cached `measureText`, state-change deduplication, zero-allocation path batching,
  `Path2D` (including SVG paths), gradients, patterns, compositing, `drawImage`,
  `getImageData`/`putImageData`, `toDataURL`, and embedded fonts.
- **WebGL 1 & 2 and WebGPU:** Available on **Windows** (desktop WGL OpenGL; Vulkan/D3D12
  WebGPU) and **Linux** (EGL OpenGL; Vulkan WebGPU), with shared `webgl_shim.js` also
  powering Android EGL OpenGL ES 3 in SoftN Mobile. Current Three.js (`WebGL2`) scenes —
  `.glb`/`.gltf` models, `.hdr` IBL environments, PBR materials, SVG/image/canvas textures,
  lighting, shadow maps, instancing, transparency, and `OrbitControls` — run natively via
  `Request`, binary `Blob`/`URL.createObjectURL`, and `createImageBitmap`.
- **Shadow DOM & Constructable Stylesheets:** `attachShadow`, scoped `CSSStyleSheet`
  (`adoptedStyleSheets`), full `CSSStyleDeclaration` (`cssText`, `!important`, custom
  properties), `html5ever`-backed `DOMParser`, `XMLSerializer`, and Shadow-DOM-crossing
  form/event dispatch (`composedPath`, `.form`, `FormData`, `checkValidity`).
- **ES Modules & Import Maps:** Inline and external `<script type="module">` graphs and
  `<script type="importmap">` are parsed via `oxc` and linked automatically.

## Beyond HTML

Non-HTML navigations are handled natively by MIME type, extension, and magic bytes:

- **PDF** — rasterized per page with pure-Rust [hayro](https://github.com/LaurenzV/hayro) in a built-in viewer with a Save button.
- **Images** — displayed centered on their own page.
- **Text, JSON, XML** — rendered as wrapped monospace text.
- **Downloads** — saved safely to `Downloads` without overwriting existing files.

## Fonts

The 12 Liberation TTF faces (`Liberation Sans`, `Liberation Serif`, `Liberation Mono` in
regular/bold/italic/bold-italic) are compiled directly into `zipp-dom`, providing metric-compatible
fallbacks for Arial, Times New Roman, and Courier New on every platform (including headless
Android and RDP). On desktop builds with `system_fonts`, installed OS fonts take priority
unless `ZIPP_EMBEDDED_FONTS_ONLY=1` is set.

## Cookies and Site Storage

Profiles live in `%LOCALAPPDATA%\Zipp` on Windows (`$XDG_DATA_HOME/zipp` elsewhere, or
`ZIPP_PROFILE_DIR`):

- **Cookies:** RFC 6265 cookie jar (`cookies.json`) shared across requests and `document.cookie`, respecting `HttpOnly` and session lifetime.
- **IndexedDB & `localStorage`:** Per-origin storage under `storage/` with full IndexedDB object stores, indexes, cursors, key ranges, upgrades, and transactions (`idb`, `Dexie`, `localforage`).

## Patched Stylo & Taffy

`third_party/stylo` and `third_party/taffy` carry small targeted patches wired via
`[patch.crates-io]` and `[patch."https://github.com/DioxusLabs/taffy"]` (dark `color-scheme`
support, `@container` queries, and grid/layout fixes). See `third_party/stylo/ZIPP_PATCHES.md`
and `third_party/taffy/ZIPP_PATCHES.md` for details.

## Known Gaps

- **`-webkit-line-clamp` and `display: -webkit-box`:** Compiled out of Stylo's Servo mode; clamped text renders in full.
- **Server-rendered pages without `#root` or `<canvas>`:** Scripts still run (timers, console), while DOM replacements activate when a script populates `#root` or renders to a `<canvas>`.
- **Worker termination:** `worker.terminate()` takes effect between tasks; workers do not yet support nested workers, transferable buffers, or IndexedDB.
- **Canvas 2D shadows & filters:** Not yet drawn; conic gradients render as their first stop.
- **`mask-image`:** Unsupported (elements paint unmasked rather than dropped).
- **Float avoidance for BFC blocks:** `overflow: hidden` beside floats can overlap rather than shifting beside/below.
- **Conic/sweep CSS gradients:** Fall back to their middle stop (`tiny-skia` has no sweep shader).
- **CSS animations:** Pages with active CSS animations resolve layout each frame while animating.
- **Address bar & client-side routing:** History `pushState` updates do not yet rewrite the toolbar URL input.
- **One-shot DOM bridge:** Script DOM mutations re-serialize and patch/parse rather than mutating live DOM nodes incrementally (see [docs/DOM_BRIDGE_DESIGN.md](docs/DOM_BRIDGE_DESIGN.md)).

## License

MIT OR Apache-2.0
