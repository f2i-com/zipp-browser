# Zipp Browser

A browser engine that runs page JavaScript on [ZIPP](https://github.com/f2i-com/zipp.org) —
a clean-sheet JS engine with a register VM and native x86-64/ARM64 JITs.

Everything below the JS is real engine code, not a toy: **Stylo** (Firefox's
style engine) for the CSS cascade, **Taffy** for flexbox/grid/block layout,
**Parley** + **Skrifa** for text shaping, and **tiny-skia** for software
rasterization. No GPU required — it renders over RDP.

```
fetch → zipp-vm runs the page's JS → Stylo cascade → Taffy layout → tiny-skia paint → winit window
```

## Status

It renders real React sites. `lance.name` (React 18 + react-router) server-renders
to ~25 KB of HTML with live event handlers attached and background prerendering of
linked pages.

<!-- A screenshot lives at docs/screenshot.png once you generate one with --screenshot. -->

## Layout

| Crate | What it does |
| --- | --- |
| `crates/zipp-dom` | DOM, HTML parsing (html5ever), Stylo↔Taffy bridge, layout, events |
| `crates/zipp-renderer` | Paint tree and the tiny-skia software rasterizer |
| `crates/zipp-shell` | winit windowing, networking, the application shell |
| `apps/browser` | The `zipp-browser` binary: JS integration, DOM shim, navigation |

`zipp-vm` comes from [ZIPP](https://github.com/f2i-com/zipp.org) as a git
dependency pinned to a release tag (`v0.0.30`), consumed through its `embed`
module — a persistent VM that is compiled once, kept alive, and re-entered to
dispatch DOM event handlers into the same global context.

## Building

```
git clone git@github.com:f2i-com/zipp-browser.git
cd zipp-browser
cargo build --release
```

Cargo fetches ZIPP itself. To move to a newer engine, change the `tag` on
`zipp-vm` in `apps/browser/Cargo.toml`.

To build against a local `zipp.org` checkout instead (working on the engine and
the browser together), keep the two checkouts side by side and put a patch in a
`.cargo/config.toml` in their shared parent directory, outside both repos:

```toml
[patch."https://github.com/f2i-com/zipp.org"]
zipp-vm = { path = "zipp.org/crates/zipp-vm" }
```

## Running

```
zipp-browser                                   # opens https://lance.name
zipp-browser https://example.com               # opens a URL

zipp-browser --screenshot out.png <url> [w] [h]   # headless render to PNG
zipp-browser --test-js                            # DOM/browser JS suite
zipp-browser --test-compat                        # language compatibility suite
```

In the window, the toolbar's back, forward and reload buttons work, as do
Alt+Left/Right, the mouse's back/forward buttons, and F5 or Ctrl+R; the arrow keys,
Page Up/Down, Space and Home/End scroll the page; Ctrl+L or F6 jumps to the address bar. Every page,
including one reached by a link or revisited from history, runs its own
JavaScript and responds to clicks.

`--screenshot` writes the post-JavaScript HTML next to the image, which is how
you tell a CSS problem from a DOM problem. Build with `--release`: a debug build
runs the JS engine and rasterizer unoptimized and is far slower.

A few environment switches make a headless render exercise what a window does:

- `ZIPP_UICLICK=x,y[;x,y…]` clicks at those client coordinates before the
  capture, through hit-testing, the event driver and the JS bridge.
- `ZIPP_RESIZE=w,h[;w,h…]` resizes the live page through each size in turn.
  Resizing to a width takes a different path from loading at it, and only the
  first is what dragging the window does.
- `ZIPP_COLOR_SCHEME=light|dark` overrides the OS colour scheme, which both
  CSS `prefers-color-scheme` and the page's `matchMedia` otherwise follow.

## Video

On Windows, `<video>` plays through Media Foundation's media engine -- the
component Windows provides for HTML media -- so it gets the system's codecs
(H.264, VP9, AV1, MPEG-4, ... in MP4, WebM and QuickTime), sound, and network
streaming. Each new frame is copied out at the size it is shown and painted
like an image, without restyling or laying out the page. Sources are tried in
order, as the spec says, falling through to the next when one will not load or
decode (Wikipedia's VP8 originals, for one). Click the video to play or pause;
with `controls`, the bar along the bottom also seeks and mutes. A muted
`autoplay` video starts by itself, one with sound waits for a click.
`ZIPP_MUTE_MEDIA=1` mutes every player (for automated runs);
`ZIPP_DEBUG_MEDIA=1` logs the engine's events. Elsewhere -- other platforms,
`--screenshot` -- a video shows its poster.

## WebAssembly, WebGL and WebGPU

`WebAssembly` is zipp-vm's own (its `web-assembly` branch, which this branch
builds against): the whole JS API, with modules run by the
[wasmi](https://github.com/wasmi-labs/wasmi) interpreter, so a page's
`instantiateStreaming(fetch("x.wasm"))` works as written.

On Windows a `<canvas>` can draw with WebGL 1 or WebGPU. The page's calls are
batched and replayed off the window's thread -- WebGL on desktop OpenGL
(`webgl.rs`), WebGPU on [wgpu](https://wgpu.rs) over Vulkan or Direct3D 12
(`webgpu.rs`) -- into an offscreen target, read back at the end of each frame
and painted as the canvas's image. The window drives `requestAnimationFrame`
while a context is alive. `getContext("webgl2")` returns null, so a page falls
back to WebGL 1.

## Beyond HTML

A navigation is classified by its `Content-Type` (then file extension, then
leading bytes), and anything that is not HTML becomes an ordinary page:

- **PDF** -- every page rasterised with [hayro](https://github.com/LaurenzV/hayro)
  (pure Rust) at the size it is shown, in a viewer with the file name, page
  count and a Save button.
- **Images** -- shown on their own, centred.
- **Text, JSON, XML** -- shown as wrapped monospace text, not run through the
  HTML parser.
- **Anything else** -- saved to `Downloads` (never overwriting: `a (1).zip`),
  with a page saying where it went.

## Fonts

The Liberation faces are compiled into the binary, so `serif`, `sans-serif` and
`monospace` always resolve and nothing is read from a system font directory.
They are metric-compatible with Arial, Times New Roman and Courier New, so
substituting them keeps advances and line breaks where the page expected them.

Installed system fonts are still discovered and take priority, so a page asking
for a font you actually have gets the real one. `ZIPP_EMBEDDED_FONTS_ONLY=1`
flips that, making rendering independent of the host — use it when comparing
screenshots taken on different machines.

## How JavaScript works

zipp-vm deliberately exposes no host surface of its own, so the browser supplies
one. A single string channel (`__zippHostCall`) backs synchronous HTTP for
`fetch` and page console output; everything else the page sees comes from
`dom_shim.js`, a JS-implemented DOM that the bundle runs against. After the
script settles, the shim serializes its tree to HTML, which is parsed into the
real DOM for styling and layout. The VM stays alive, so a click re-enters the
page's handlers and the updated tree is re-rendered.

## Patched stylo

`third_party/stylo` is stylo 0.14.0 from crates.io with one fix, wired in by
`[patch.crates-io]`: its servo build never reported a dark colour scheme, so
`light-dark()` and dark system colours were unusable. See
`third_party/stylo/ZIPP_PATCHES.md`; drop the copy when moving to a stylo that
carries the fix.

## Known gaps

Honest list, not an exhaustive one:

- **`-webkit-line-clamp` and `display:-webkit-box` do nothing.** Both are
  `engine = "gecko"` in stylo, so they are compiled out of a servo-featured
  build and no pref can reach them — the declarations are dropped in the
  cascade. Text that Chrome truncates to N lines with an ellipsis renders in
  full, making clamped cards taller. Fixing it means patching stylo, not this
  repo.
- **Scripts only take effect on pages that render into `#root`.** The shim's
  document starts as an empty `<div id="root">`, not the page's own markup, and
  its tree is used only when that root was filled. A client-rendered app works;
  on a server-rendered page the scripts run but cannot see or change its
  markup, and listeners they attach to it never fire (example.com's
  translations, a hand-written menu toggle). Seeding the shim with the parsed
  page is the fix.
- **`mask-image` is unsupported**; a masked element paints unmasked. That is
  much closer to Chrome than dropping the layer, but a `mask-image` fade (the
  hero dot-grid's, for one) simply does not fade, and an icon drawn as a
  `background-color` shaped by a mask (MDN's menu chevrons) is a solid square.
- **A block that establishes a BFC does not avoid floats.** `overflow: hidden`
  next to a float should sit beside it, or below when there is no room; instead
  it overlaps. This is what still misplaces GitHub's profile name over its
  Follow button.
- **Conic/sweep gradients** fall back to their middle stop — tiny-skia has no
  sweep shader.
- **Intrinsic sizing keywords** (`max-content`, `min-content`, `fit-content`,
  `stretch`) collapse to `auto` in the Stylo→Taffy conversion. Lowering them
  properly currently hits an `unreachable!()` in Taffy's resolver.
- **An animating page costs about one CPU core.** Without the `incremental`
  feature (which does not currently build), every frame restyles, rebuilds the
  whole box tree -- re-parsing every inline SVG -- and lays out again, so a
  page with a running CSS animation (lance.name's marquee) keeps a core busy
  for as long as it is open. Memory stays flat.
- **A window resize does not reach the page's JavaScript.** CSS re-lays out
  for the new size, but `window.innerWidth`, `matchMedia` and `resize` events
  stay at the size the page loaded with, so layout a script chooses in JS
  keeps its load-time breakpoint until the page is reloaded.
- **Client-side routing does not update the address bar.** The URL shown stays
  the one that was loaded, even after in-page navigation.
- **The DOM bridge is one-shot per render.** JS mutations are re-serialized and
  re-parsed rather than applied incrementally to the live tree. See
  `docs/DOM_BRIDGE_DESIGN.md`.
- No scrolling-driven `IntersectionObserver`: observed elements are reported as
  intersecting immediately, which matches a fully-scrolled page rather than the
  initial viewport.

## License

MIT OR Apache-2.0
