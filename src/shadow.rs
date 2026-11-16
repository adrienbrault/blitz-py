//! Box-shadow colours clamped to the sRGB gamut before vello_cpu sees them.
//!
//! blitz-paint converts a shadow's colour to sRGB without gamut mapping, so
//! a wide-gamut colour (`oklch()`, `color(display-p3 ...)`, Tailwind v4's
//! palette) can arrive with a channel below 0 or above 1. Fills tolerate
//! that, but vello_cpu's `fill_blurred_rounded_rect` (every blurred or
//! spread `box-shadow`) does not: a channel at -0.2 paints at full
//! intensity, so an emerald `oklch(76.5% 0.177 163.2)` glow comes out red.
//! [`ClampShadows`] wraps the scene blitz-paint draws into and clamps
//! `draw_box_shadow`'s colour per channel, which is what fills effectively
//! do. Everything else is forwarded untouched. See docs/UPSTREAM.md.

use std::sync::Arc;

use anyrender::{
    Filter, Glyph, NormalizedCoord, PaintRef, PaintScene, RegisterResourceError, RenderContext,
    ResourceId,
};
use peniko::kurbo::{Affine, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};

pub(crate) struct ClampShadows<'p, S: PaintScene>(pub(crate) &'p mut S);

/// `color` with each colour channel clamped to 0..=1 (alpha untouched).
pub(crate) fn clamp_to_srgb(color: Color) -> Color {
    let [r, g, b, a] = color.components;
    Color::new([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a])
}

impl<S: PaintScene> RenderContext for ClampShadows<'_, S> {
    fn try_register_custom_resource(
        &mut self,
        resource: Box<dyn std::any::Any>,
    ) -> Result<ResourceId, RegisterResourceError> {
        self.0.try_register_custom_resource(resource)
    }

    fn unregister_resource(&mut self, resource_id: ResourceId) {
        self.0.unregister_resource(resource_id);
    }

    fn renderer_specific_context(&self) -> Option<Box<dyn std::any::Any>> {
        self.0.renderer_specific_context()
    }
}

impl<S: PaintScene> PaintScene for ClampShadows<'_, S> {
    fn reset(&mut self) {
        self.0.reset();
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        filter: Option<Arc<Filter>>,
        backdrop_filter: Option<Arc<Filter>>,
    ) {
        self.0
            .push_layer(fill, blend, alpha, transform, clip, filter, backdrop_filter);
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &impl Shape) {
        self.0.push_clip_layer(fill, transform, clip);
    }

    fn pop_layer(&mut self) {
        self.0.pop_layer();
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.0
            .stroke(style, transform, brush, brush_transform, shape);
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
    ) {
        self.0.fill(style, transform, brush, brush_transform, shape);
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        embolden: Vec2,
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph> + Clone,
    ) {
        self.0.draw_glyphs(
            font,
            font_size,
            hint,
            normalized_coords,
            embolden,
            style,
            brush,
            brush_alpha,
            transform,
            glyph_transform,
            glyphs,
        );
    }

    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        rect: Rect,
        brush: Color,
        radius: f64,
        std_dev: f64,
    ) {
        self.0
            .draw_box_shadow(transform, rect, clamp_to_srgb(brush), radius, std_dev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_colour_channels_but_not_alpha() {
        let clamped = clamp_to_srgb(Color::new([-0.2, 0.8, 1.3, 0.5]));
        assert_eq!(clamped.components, [0.0, 0.8, 1.0, 0.5]);
    }
}
