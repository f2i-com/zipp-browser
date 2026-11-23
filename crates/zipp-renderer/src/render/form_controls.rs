//! Native form control rendering (checkboxes, radio buttons, select dropdowns).
use super::ElementCx;
use crate::color::{Color, ToColorColor as _};
use anyrender::PaintScene;
use zipp_dom::local_name;
use kurbo::{Affine, BezPath, Cap, Circle, Join, Point, RoundedRect, Stroke, Vec2};
use peniko::Fill;
use style::dom::TElement as _;

impl ElementCx<'_> {
    /// A select's arrow, at its right, in its text colour -- the native one,
    /// unless the page draws its own as a background image, which is how a
    /// page that turns the native one off (`appearance: none`) replaces it.
    /// (`appearance` itself is Gecko's alone in this Stylo.)
    pub(super) fn draw_select_arrow(&self, scene: &mut impl PaintScene) {
        use style::values::generics::image::GenericImage;
        if self.node.local_name() != "select" {
            return;
        }
        let background = self.style.get_background();
        if background.background_image.0.iter().any(|image| !matches!(image, GenericImage::None)) {
            return;
        }
        let color = self.style.clone_color().as_srgb_color();
        let frame = self.frame.border_box;
        let s = self.scale;
        if frame.width() < 24.0 * s || frame.height() < 8.0 * s {
            return;
        }
        let (cx, cy) = (frame.x1 - 13.0 * s, frame.center().y);
        // Mask the right arrow gutter with the select's own background color so
        // long option labels (when author CSS sets small padding-right) never
        // collide visually with the dropdown chevron.
        let bg = background
            .background_color
            .resolve_to_absolute(&self.style.clone_color())
            .as_srgb_color();
        if bg.components[3] > 0.05 {
            let gutter = self.frame.padding_box;
            let mask_x0 = (frame.x1 - 24.0 * s).max(gutter.x0);
            if gutter.x1 > mask_x0 && gutter.y1 > gutter.y0 {
                let mask_rect = kurbo::Rect::new(mask_x0, gutter.y0, gutter.x1, gutter.y1);
                scene.fill(Fill::NonZero, self.transform, bg, None, &mask_rect);
            }
        }
        let (w, h) = (4.0 * s, 2.4 * s);
        let mut path = BezPath::new();
        path.move_to((cx - w, cy - h));
        path.line_to((cx, cy + h));
        path.line_to((cx + w, cy - h));
        let stroke = Stroke::new(1.6 * s).with_caps(Cap::Round).with_join(Join::Round);
        scene.stroke(&stroke, self.transform, color, None, &path);
    }

    pub(super) fn draw_input(&self, scene: &mut impl PaintScene) {
        if self.node.local_name() != "input" {
            return;
        }
        let Some(checked) = self.element.checkbox_input_checked() else {
            return;
        };

        let type_attr = self.node.attr(local_name!("type"));
        let disabled = self.node.attr(local_name!("disabled")).is_some();

        // TODO this should be coming from css accent-color, but I couldn't find how to retrieve it
        let accent_color = if disabled {
            Color::from_rgba8(209, 209, 209, 255)
        } else {
            self.style.clone_color().as_srgb_color()
        };

        let width = self.frame.border_box.width();
        let height = self.frame.border_box.height();
        let min_dimension = width.min(height);
        let scale = (min_dimension - 4.0).max(0.0) / 16.0;

        let frame = self.frame.border_box.to_rounded_rect(scale * 2.0);

        match type_attr {
            Some("checkbox") => {
                draw_checkbox(scene, checked, frame, self.transform, accent_color, scale);
            }
            Some("radio") => {
                let center = frame.center();
                draw_radio_button(scene, checked, center, self.transform, accent_color, scale);
            }
            _ => {}
        }
    }
}

fn draw_checkbox(
    scene: &mut impl PaintScene,
    checked: bool,
    frame: RoundedRect,
    transform: Affine,
    accent_color: Color,
    scale: f64,
) {
    let c = accent_color.to_rgba8();
    let lum = 0.2126 * (c.r as f32 / 255.0)
        + 0.7152 * (c.g as f32 / 255.0)
        + 0.0722 * (c.b as f32 / 255.0);
    if checked {
        scene.fill(Fill::NonZero, transform, accent_color, None, &frame);
        //Tick code derived from masonry
        let mut path = BezPath::new();
        path.move_to((2.0, 9.0));
        path.line_to((6.0, 13.0));
        path.line_to((14.0, 2.0));

        path.apply_affine(Affine::translate(Vec2 { x: 2.0, y: 1.0 }).then_scale(scale));

        let style = Stroke {
            width: 2.0 * scale,
            join: Join::Round,
            miter_limit: 10.0,
            start_cap: Cap::Round,
            end_cap: Cap::Round,
            dash_pattern: Default::default(),
            dash_offset: 0.0,
        };

        let tick_color = if lum > 0.7 {
            Color::from_rgba8(15, 23, 42, 255)
        } else {
            Color::WHITE
        };
        scene.stroke(&style, transform, tick_color, None, &path);
    } else {
        let (bg, border) = if lum > 0.55 {
            (
                Color::from_rgba8(15, 23, 42, 210),
                Color::from_rgba8(148, 163, 184, 200),
            )
        } else {
            (Color::WHITE, accent_color)
        };
        scene.fill(Fill::NonZero, transform, bg, None, &frame);
        scene.stroke(&Stroke::new(scale.max(1.0)), transform, border, None, &frame);
    }
}

fn draw_radio_button(
    scene: &mut impl PaintScene,
    checked: bool,
    center: Point,
    transform: Affine,
    accent_color: Color,
    scale: f64,
) {
    let outer_ring = Circle::new(center, 8.0 * scale);
    let gap = Circle::new(center, 6.0 * scale);
    let inner_circle = Circle::new(center, 4.0 * scale);
    if checked {
        scene.fill(Fill::NonZero, transform, accent_color, None, &outer_ring);
        scene.fill(Fill::NonZero, transform, Color::WHITE, None, &gap);
        scene.fill(Fill::NonZero, transform, accent_color, None, &inner_circle);
    } else {
        const GRAY: Color = color::palette::css::GRAY;
        scene.fill(Fill::NonZero, transform, GRAY, None, &outer_ring);
        scene.fill(Fill::NonZero, transform, Color::WHITE, None, &gap);
    }
}
