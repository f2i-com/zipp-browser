//! Box shadow rendering (inset and outset).
use super::ElementCx;
use crate::color::ToColorColor as _;
use anyrender::PaintScene;
use kurbo::{Rect, RoundedRect, Shape as _, Vec2};
use peniko::{Fill, Mix};

impl ElementCx<'_> {
    pub(super) fn draw_outset_box_shadow(&self, scene: &mut impl PaintScene) {
        let box_shadow = &self.style.get_effects().box_shadow.0;

        // TODO: Only apply clip if element has transparency
        let has_outset_shadow = box_shadow.iter().any(|s| !s.inset);
        if !has_outset_shadow {
            return;
        }

        let current_color = self.style.clone_color();
        let max_shadow_rect = box_shadow.iter().fold(Rect::ZERO, |prev, shadow| {
            let x = shadow.base.horizontal.px() as f64 * self.scale;
            let y = shadow.base.vertical.px() as f64 * self.scale;
            let blur = shadow.base.blur.px() as f64 * self.scale;
            let spread = shadow.spread.px() as f64 * self.scale;
            let offset = spread + blur * 2.5;

            let rect = self.frame.border_box.inflate(offset, offset) + Vec2::new(x, y);

            prev.union(rect)
        });

        self.context.layer_manager.maybe_with_layer(
            scene,
            has_outset_shadow,
            1.0,
            self.transform,
            &self.frame.shadow_clip(max_shadow_rect),
            |scene| {
                for shadow in box_shadow.iter().filter(|s| !s.inset).rev() {
                    let shadow_color = shadow
                        .base
                        .color
                        .resolve_to_absolute(&current_color)
                        .as_srgb_color();

                    let alpha = shadow_color.components[3];
                    if alpha != 0.0 {
                        let transform = self.transform.then_translate(Vec2 {
                            x: shadow.base.horizontal.px() as f64 * self.scale,
                            y: shadow.base.vertical.px() as f64 * self.scale,
                        });

                        // TODO draw shadows with matching individual radii instead of averaging
                        let radius = self.frame.border_radii.average();

                        let spread = shadow.spread.px() as f64 * self.scale;
                        let rect = self.frame.border_box.inflate(spread, spread);

                        // Fill the color
                        scene.draw_box_shadow(
                            transform,
                            rect,
                            shadow_color,
                            radius,
                            shadow.base.blur.px() as f64,
                        );
                    }
                }
            },
        )
    }

    /// Inset shadows, inside the padding box: its area less a hole -- the
    /// padding box moved by the shadow's offset and shrunk by its spread --
    /// whose edge the blur softens. Painted as plain fills, rings for the
    /// blur, rather than a layer composited `DestOut`, which the CPU painter
    /// does not do: it drew the hole in white over the element, so an inset
    /// shadow (a button's `inset 0 1px #fff3` highlight, Tailwind's inset
    /// rings) hid its element under a white box.
    pub(super) fn draw_inset_box_shadow(&self, scene: &mut impl PaintScene) {
        let current_color = self.style.clone_color();
        let box_shadow = &self.style.get_effects().box_shadow.0;
        if !box_shadow.iter().any(|s| s.inset) {
            return;
        }

        let padding_box = self.frame.padding_box_path();
        let border = self.frame.border_width;
        // TODO draw shadows with matching individual radii instead of averaging
        // The padding box's corners: the border's, less the border.
        let radius = (self.frame.border_radii.average() - (border.x0 + border.x1 + border.y0 + border.y1) / 4.0).max(0.0);

        scene.push_layer(Mix::Normal, 1.0, self.transform, &padding_box);
        // The first shadow is on top.
        for shadow in box_shadow.iter().filter(|s| s.inset).rev() {
            let color = shadow.base.color.resolve_to_absolute(&current_color).as_srgb_color();
            let alpha = f64::from(color.components[3]);
            if alpha == 0.0 {
                continue;
            }
            let x = shadow.base.horizontal.px() as f64 * self.scale;
            let y = shadow.base.vertical.px() as f64 * self.scale;
            let blur = (shadow.base.blur.px() as f64 * self.scale).max(0.0);
            let spread = shadow.spread.px() as f64 * self.scale;
            let hole = self.frame.padding_box + Vec2::new(x, y);
            let reach = x.abs() + y.abs() + spread.abs() + blur + 1.0;
            let outer = self.frame.padding_box.inflate(reach, reach);

            // The blur: rings whose hole edges run from `spread + blur` in to
            // `spread - blur`, the smallest hole first. A point is under as
            // many rings as it is deep in the shadow, and the k-th ring's
            // alpha brings what is under k of them to k/n of the colour's.
            let rings = if blur > 0.0 { (blur / 2.0).ceil().clamp(2.0, 6.0) as usize } else { 1 };
            for k in 1..=rings {
                let inset = if rings == 1 {
                    spread
                } else {
                    spread + blur * (1.0 - 2.0 * (k - 1) as f64 / (rings - 1) as f64)
                };
                let below = 1.0 - alpha * (k - 1) as f64 / rings as f64;
                let ring_alpha = 1.0 - (1.0 - alpha * k as f64 / rings as f64) / below;
                let ring_hole = hole.inflate(-inset, -inset);
                let mut path = outer.to_path(0.1);
                if ring_hole.width() > 0.0 && ring_hole.height() > 0.0 {
                    path.extend(RoundedRect::from_rect(ring_hole, (radius - inset).max(0.0)).path_elements(0.1));
                }
                scene.fill(Fill::EvenOdd, self.transform, color.with_alpha(ring_alpha as f32), None, &path);
            }
        }
        scene.pop_layer();
    }
}
