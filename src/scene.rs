//! The scene blitz-paint draws into, adapted to what vello_cpu can paint.
//!
//! [`EngineScene`] wraps the rasterizer's scene and forwards every call,
//! except where blitz-paint hands vello_cpu something it mishandles:
//!
//! - **Box-shadow colours outside sRGB.** blitz-paint converts a shadow's
//!   colour to sRGB without gamut mapping, so a wide-gamut colour
//!   (`oklch()`, `color(display-p3 ...)`, Tailwind v4's palette) can
//!   arrive with a channel below 0 or above 1. Fills tolerate that, but
//!   vello_cpu's `fill_blurred_rounded_rect` (every blurred or spread
//!   `box-shadow`) does not: a channel at -0.2 paints at full intensity,
//!   so an emerald `oklch(76.5% 0.177 163.2)` glow comes out red. The
//!   colour is clamped per channel, which is what fills effectively do.
//! - **Filters vello_cpu doesn't implement.** With anyrender_vello_cpu's
//!   `filters` feature, CSS `filter: blur()` / `drop-shadow()` render, but
//!   the colour filters (`grayscale()`, `sepia()`, `brightness()`, ...)
//!   become colour-matrix primitives that vello_cpu answers with
//!   `unimplemented!()`: a panic, i.e. a `PanicException` in Python. Only
//!   filters whose (first, and only used) primitive vello_cpu implements are
//!   passed on; the others are dropped, as they were before filters were on.
//!
//! See docs/UPSTREAM.md.

use std::sync::Arc;

use anyrender::filters::FilterEffect;
use anyrender::{
    Filter, Glyph, NormalizedCoord, PaintRef, PaintScene, RegisterResourceError, RenderContext,
    ResourceId,
};
use peniko::kurbo::{Affine, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, StyleRef};

pub(crate) struct EngineScene<'p, S: PaintScene>(pub(crate) &'p mut S);

/// `color` with each colour channel clamped to 0..=1 (alpha untouched).
pub(crate) fn clamp_to_srgb(color: Color) -> Color {
    let [r, g, b, a] = color.components;
    Color::new([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a])
}

/// `filter` when vello_cpu can paint it, else None.
///
/// anyrender_vello_cpu 0.18 converts only a filter's first node, and
/// vello_cpu 0.3 implements flood, Gaussian blur, drop shadow and offset;
/// anything else reaches an `unimplemented!()`.
pub(crate) fn paintable(filter: Option<Arc<Filter>>) -> Option<Arc<Filter>> {
    let first = filter.as_ref()?.nodes().first()?;
    match first.effect {
        FilterEffect::Flood(_)
        | FilterEffect::GaussianBlur(_)
        | FilterEffect::DropShadow(_)
        | FilterEffect::Offset(_) => filter,
        _ => None,
    }
}

impl<S: PaintScene> RenderContext for EngineScene<'_, S> {
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

impl<S: PaintScene> PaintScene for EngineScene<'_, S> {
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
        self.0.push_layer(
            fill,
            blend,
            alpha,
            transform,
            clip,
            paintable(filter),
            backdrop_filter,
        );
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
    fn keeps_only_filters_vello_cpu_paints() {
        let one = |effect: FilterEffect| Some(Arc::new(Filter::linear_list([effect].into_iter())));
        assert!(paintable(one(FilterEffect::blur(4.0))).is_some());
        assert!(paintable(one(FilterEffect::grayscale(1.0))).is_none());
        assert!(paintable(one(FilterEffect::sepia(1.0))).is_none());
        assert!(paintable(None).is_none());
    }

    #[test]
    fn clamps_colour_channels_but_not_alpha() {
        let clamped = clamp_to_srgb(Color::new([-0.2, 0.8, 1.3, 0.5]));
        assert_eq!(clamped.components, [0.0, 0.8, 1.0, 0.5]);
    }
}
