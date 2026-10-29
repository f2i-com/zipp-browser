# taffy at df2663a, patched for zipp-browser

This is Taffy from `https://github.com/DioxusLabs/taffy` at rev
`df2663aad35801f7522f6878a0300646dfe624f7` -- the rev the workspace names --
unchanged except as listed here. The workspace uses it through
`[patch."https://github.com/DioxusLabs/taffy"]` in the root `Cargo.toml`.
Only `src/`, `Cargo.toml`, `README.md` and `LICENSE.md` are kept; the
manifest's dev-dependencies, examples, benches and workspace are gone, as
they name files not copied. To move to a newer Taffy, drop this directory and
the patch entry once upstream carries the same behaviour.

## `compute/block.rs`: percentages of an `auto` height

A block resolved its children's percentage heights against its `min-height`
when its own height was `auto`. Browsers treat the percentage as `auto` there:
a height is definite only when it is given. A box with `min-height: 0` (GitHub
writes `min-block-size: 0`) gave a `height: 100%` child a height of nothing,
and its content -- GitHub's home page hero -- overflowed out of a collapsed
box.

## `width: min-content | max-content | fit-content` on block children

`Dimension` has no intrinsic sizing keywords, so these reached Taffy as
`auto` and a block child with one was stretched to the container's width.
`Style::item_intrinsic_width` (`BlockItemStyle::intrinsic_width`) now says
which keyword the width was, and block layout sizes such a child to its
content -- `fit-content` to its max-content width, but no wider than the
space available or narrower than its min-content width -- then places it
with its margins, so `margin: auto` centres it. When a container is sized to
its content, `min-content` and `max-content` children contribute that size.

GitHub's activity strip is a `width: max-content` grid of columns; stretched
to the window, each column was a few characters wide and the text ran down
the page.
