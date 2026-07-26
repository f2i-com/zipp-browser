# Zipp Browser

A browser engine that runs page JavaScript on [zipp-lang](https://github.com/izuc/zipp-lang) —
a clean-sheet JS engine with a register VM and a native x86-64 JIT.

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

`zipp-vm` comes from the sibling [`zipp-lang`](https://github.com/izuc/zipp-lang)
checkout via a path dependency, consumed through its `embed` module — a
persistent VM that is compiled once, kept alive, and re-entered to dispatch DOM
event handlers into the same global context.

## Building

Needs a `zipp-lang` checkout beside this one:

```
git clone git@github.com:izuc/zipp-lang.git
git clone git@github.com:izuc/zipp-browser.git
cd zipp-browser
cargo build --release
```

## Running

```
zipp-browser                                   # opens https://lance.name
zipp-browser https://example.com               # opens a URL

zipp-browser --screenshot out.png <url> [w] [h]   # headless render to PNG
zipp-browser --test-js                            # DOM/browser JS suite
zipp-browser --test-compat                        # language compatibility suite
```

`--screenshot` writes the post-JavaScript HTML next to the image, which is how
you tell a CSS problem from a DOM problem. Build with `--release`: a debug build
runs the JS engine and rasterizer unoptimized and is far slower.

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

## Known gaps

Honest list, not an exhaustive one:

- **`-webkit-line-clamp` and `display:-webkit-box` do nothing.** Both are
  `engine = "gecko"` in stylo, so they are compiled out of a servo-featured
  build and no pref can reach them — the declarations are dropped in the
  cascade. Text that Chrome truncates to N lines with an ellipsis renders in
  full, making clamped cards taller. Fixing it means patching stylo, not this
  repo.
- **`mask-image` is unsupported**; a masked element paints unmasked. That is
  much closer to Chrome than dropping the layer, but a `mask-image` fade (the
  hero dot-grid's, for one) simply does not fade.
- **Conic/sweep gradients** fall back to their middle stop — tiny-skia has no
  sweep shader.
- **Intrinsic sizing keywords** (`max-content`, `min-content`, `fit-content`,
  `stretch`) collapse to `auto` in the Stylo→Taffy conversion. Lowering them
  properly currently hits an `unreachable!()` in Taffy's resolver.
- **A real navigation drops the page's JavaScript.** In-page routing is fine —
  a framework that cancels the click (React Router does) keeps its live VM, so
  SPA links work and the page stays interactive. But an address-bar navigation
  or a link the page does not handle replaces the document with fetched or
  prerendered HTML and installs no event bridge, so that page renders correctly
  and responds to nothing. Fixing it means running the new page's JS and
  attaching a bridge to the new document in the navigation path.
- **Client-side routing does not update the address bar.** The URL shown stays
  the one that was loaded, even after in-page navigation.
- **The DOM bridge is one-shot per render.** JS mutations are re-serialized and
  re-parsed rather than applied incrementally to the live tree. See
  `docs/DOM_BRIDGE_DESIGN.md`.
- **Builtin statics ignore reassignment in zipp-vm** — `Object.keys = f` then
  `Object.keys(x)` still calls the native, because the call site is lowered at
  compile time. This breaks polyfills, and is why 4 of the bundled JS tests
  fail.
- `Error.prototype.stack` is empty in zipp-vm, so page errors have no traceback.
- No scrolling-driven `IntersectionObserver`: observed elements are reported as
  intersecting immediately, which matches a fully-scrolled page rather than the
  initial viewport.

## License

MIT OR Apache-2.0
