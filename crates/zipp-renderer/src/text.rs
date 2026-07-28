//! Text rendering: glyph drawing, underlines, strikethroughs, and selection highlights.

use anyrender::PaintScene;
use zipp_dom::{BaseDocument, node::TextBrush, util::ToColorColor};
use kurbo::{Affine, Stroke};
use parley::{Affinity, Cursor, Layout, Line, PositionedLayoutItem, Selection};
use peniko::Fill;
use style::values::computed::TextDecorationLine;
use style::values::specified::box_::{DisplayInside, DisplayOutside};
use crate::color::ToColorColor as _;

use crate::SELECTION_COLOR;

/// Paint the background of non-replaced INLINE elements.
///
/// An inline element never becomes a Taffy node — `build_inline_layout_recursive`
/// turns it into a parley style span carrying font and colour only — so the
/// ordinary box-painting path never runs for it. `<span style="background:…">`,
/// a highlighted `<code>`, a coloured `<mark>` and every "badge" span painted
/// nothing at all.
///
/// The element's node id rides on every glyph run via [`TextBrush`], so the run
/// geometry parley has already computed is enough to rebuild the fragment
/// boxes. Runs belonging to the same element on the same line are merged, so a
/// fragment split across several runs — a font fallback, a bidi boundary — does
/// not show seams between them.
///
/// Only elements whose own display is inline-flow are painted. Text sitting
/// directly in a block carries that BLOCK's id in its brush, and the block has
/// already drawn its own background through the normal path; painting it again
/// here would double it up.
///
/// Padding and borders are not applied: inline layout reserves no space for
/// either, so honouring them would overlap the neighbouring text rather than
/// displace it. The background covers the content area — the font's ascent to
/// its descent — which is where a browser puts it.
pub(crate) fn draw_inline_backgrounds<'a>(
    scene: &mut impl PaintScene,
    lines: impl Iterator<Item = Line<'a, TextBrush>>,
    doc: &BaseDocument,
    transform: Affine,
) {
    // (node id, x0, x1, top, bottom)
    let mut frags: Vec<(usize, f64, f64, f64, f64)> = Vec::new();

    for line in lines {
        frags.clear();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let metrics = glyph_run.run().metrics();
            let id = glyph_run.style().brush.id;
            let x0 = glyph_run.offset() as f64;
            let x1 = x0 + glyph_run.advance() as f64;
            let baseline = glyph_run.baseline() as f64;
            let top = baseline - metrics.ascent as f64;
            let bottom = baseline + metrics.descent as f64;

            match frags.last_mut() {
                // Same element, and this run starts where the last one ended.
                Some(last) if last.0 == id && (x0 - last.2).abs() < 0.5 => {
                    last.2 = x1;
                    last.3 = last.3.min(top);
                    last.4 = last.4.max(bottom);
                }
                _ => frags.push((id, x0, x1, top, bottom)),
            }
        }

        for &(id, x0, x1, top, bottom) in frags.iter() {
            if x1 - x0 <= 0.0 {
                continue;
            }
            let Some(styles) = doc.get_node(id).and_then(|n| n.primary_styles()) else {
                continue;
            };
            let display = styles.clone_display();
            if display.outside() != DisplayOutside::Inline
                || display.inside() != DisplayInside::Flow
            {
                continue;
            }
            let current_color = styles.clone_color();
            let bg = styles
                .clone_background_color()
                .resolve_to_absolute(&current_color)
                .as_srgb_color();
            if bg.components[3] <= 0.0 {
                continue;
            }
            let rect = kurbo::Rect::new(x0, top, x1, bottom);
            scene.fill(Fill::NonZero, transform, bg, None, &rect);
        }
    }
}

/// Draws the glyph runs for every line in the provided iterator, including
/// underline and strikethrough decorations when the computed style requests them.
pub(crate) fn draw_text_runs<'a>(
    scene: &mut impl PaintScene,
    lines: impl Iterator<Item = Line<'a, TextBrush>>,
    doc: &BaseDocument,
    transform: Affine,
) {
    for line in lines {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let font = run.font();
                let font_size = run.font_size();
                let metrics = run.metrics();
                let style = glyph_run.style();
                let synthesis = run.synthesis();
                let glyph_xform = synthesis
                    .skew()
                    .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));

                // Styles
                let styles = doc
                    .get_node(style.brush.id)
                    .unwrap()
                    .primary_styles()
                    .unwrap();
                let itext_styles = styles.get_inherited_text();
                let text_styles = styles.get_text();
                let text_color = itext_styles.color.as_color_color();
                let text_decoration_color = text_styles
                    .text_decoration_color
                    .as_absolute()
                    .map(ToColorColor::as_color_color)
                    .unwrap_or(text_color);
                let text_decoration_brush = anyrender::Paint::from(text_decoration_color);
                let text_decoration_line = text_styles.text_decoration_line;
                let has_underline = text_decoration_line.contains(TextDecorationLine::UNDERLINE);
                let has_strikethrough =
                    text_decoration_line.contains(TextDecorationLine::LINE_THROUGH);

                scene.draw_glyphs(
                    font,
                    font_size,
                    true, // hint
                    run.normalized_coords(),
                    Fill::NonZero,
                    &anyrender::Paint::from(text_color),
                    1.0, // alpha
                    transform,
                    glyph_xform,
                    glyph_run.positioned_glyphs().map(|glyph| anyrender::Glyph {
                        id: glyph.id as _,
                        x: glyph.x,
                        y: glyph.y,
                    }),
                );

                let mut draw_decoration_line =
                    |offset: f32, size: f32, brush: &anyrender::Paint| {
                        let x = glyph_run.offset() as f64;
                        let w = glyph_run.advance() as f64;
                        let y = (glyph_run.baseline() - offset + size / 2.0) as f64;
                        let line = kurbo::Line::new((x, y), (x + w, y));
                        scene.stroke(&Stroke::new(size as f64), transform, brush, None, &line)
                    };

                if has_underline {
                    let offset = metrics.underline_offset;
                    let size = metrics.underline_size;

                    // TODO: intercept line when crossing an descending character like "gqy"
                    draw_decoration_line(offset, size, &text_decoration_brush);
                }
                if has_strikethrough {
                    let offset = metrics.strikethrough_offset;
                    let size = metrics.strikethrough_size;

                    draw_decoration_line(offset, size, &text_decoration_brush);
                }
            }
        }
    }
}

/// Draw selection highlight rectangles for the given byte range in a layout.
/// Uses Parley's Selection type for accurate geometry calculation.
pub(crate) fn draw_text_selection(
    scene: &mut impl PaintScene,
    layout: &Layout<TextBrush>,
    transform: Affine,
    selection_start: usize,
    selection_end: usize,
) {
    let anchor = Cursor::from_byte_index(layout, selection_start, Affinity::Downstream);
    let focus = Cursor::from_byte_index(layout, selection_end, Affinity::Downstream);
    let selection = Selection::new(anchor, focus);

    selection.geometry_with(layout, |rect, _line_idx| {
        let rect = kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1);
        scene.fill(Fill::NonZero, transform, SELECTION_COLOR, None, &rect);
    });
}
