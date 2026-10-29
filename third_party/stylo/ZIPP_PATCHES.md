# stylo 0.14.0, patched for zipp-browser

This is the `stylo` 0.14.0 crate from crates.io, unchanged except as listed
here. The workspace uses it through `[patch.crates-io]` in the root
`Cargo.toml`. To move to a newer stylo, drop this directory and the patch
entry once upstream carries the same behaviour (it does from about 0.15 on,
except for the `normal` difference noted below).

## `device/servo.rs`: `Device::is_dark_color_scheme`

Upstream 0.14 hard-wired this to `false` for servo builds, so `light-dark()`
always resolved to its light colour and dark system colours (`Canvas`,
`CanvasText`, ...) were never used: a page styled with
`color-scheme: light dark` stayed light in dark mode.

It now follows upstream's later logic: a scheme supporting only one of light
and dark uses that one; one supporting both follows the user's preference.
Unlike upstream, `normal` (neither) stays light, as in Chrome.

## Container queries outside Gecko

Upstream 0.14 parsed `@container` rules (`stylesheets/rule_parser.rs`), the
container-relative lengths `cqw`/`cqh`/`cqi`/`cqb`/`cqmin`/`cqmax`
(`values/specified/length.rs`) and the `container` shorthand
(`properties/shorthands.toml`, `properties/shorthands.rs`) only in Gecko
builds, although `container-type` and `container-name` were available to servo
behind `layout.container-queries.enabled`. A servo embedder could make an
element a container and then use nothing that depends on it.

All of them now follow that one pref (`container_queries_enabled()` in
`stylesheets/container_rule.rs`); Gecko builds are unchanged. Evaluating them
was already engine-neutral: it asks the embedder for a container's size
through `TElement::query_container_size`, which zipp-dom answers from layout.
