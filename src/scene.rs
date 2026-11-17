//! The scene blitz-paint draws into, adapted to what vello_cpu can paint.
//!
//! [`EngineScene`] wraps the rasterizer's scene and forwards every call,
//! except where blitz-paint hands vello_cpu something it mishandles:
//!
//! - **Box-shadow colours outside sRGB.** blitz-paint converts a shadow's
//!   colour to sRGB without gamut mapping, so a wide-gamut colour
//!   (`oklch()`, `color(display-p3 ...)`, Tailwind v4's palette) can
//!   arrive with a channel below 0 or above 1. vello_cpu's blurred
//!   rounded-rect paint (every blurred `box-shadow`) converts the raw
//!   floats to bytes without clamping: a channel at -0.2 paints at full
//!   intensity on x86, so an emerald `oklch(76.5% 0.177 163.2)` glow comes
//!   out red, and a channel above 1 wraps dark. The colour is clamped per
//!   channel before premultiplication, which is what a gamut clip does.
//!
//! - **Filter regions.** anyrender_vello_cpu clips a filter layer to its
//!   filter region with a non-isolated clip (`push_clip_path`) around the
//!   layer, and vello_cpu applies a non-isolated clip enclosing a filter
//!   layer displaced down/right by the layer's filter padding. So a
//!   `filter: blur()`, a blurred `drop-shadow()` or a blurred `text-shadow`
//!   lost its halo above and left of the element. The region is clipped
//!   here with an isolated clip layer, which vello_cpu places correctly, and
//!   the filter handed on carries an unbounded region instead.
//!
//! - **Per-platform glyph snapping.** anyrender_vello_cpu 0.19 snaps glyph
//!   origins to CoreGraphics' subpixel grid when built for Apple targets
//!   and to Skia's quarter pixel elsewhere, so the same document rendered
//!   differently on macOS. It skips snapping for skewed transforms; glyph
//!   runs are drawn with a negligible horizontal skew so that text is
//!   positioned continuously, as it was before, on every platform.
//!
//! The colour filters need no guard any more: anyrender_vello_cpu 0.19
//! converts only the primitives vello_cpu implements (flood, blur, drop
//! shadow, offset) and drops the rest itself, where 0.18 reached an
//! `unimplemented!()`.
//!
//! See docs/UPSTREAM.md.

use std::sync::Arc;

use anyrender::filters::{FilterEffect, FilterGraphNode};
use anyrender::{
    AntiAliasing, BoxShadowKind, Filter, Glyph, MaskMode, NonUniformRoundedRect, NormalizedCoord,
    PaintRef, PaintScene, RegisterResourceError, RenderContext, ResourceId,
};
use peniko::kurbo::{Affine, BezPath, Rect, Shape, Stroke, Vec2};
use peniko::{BlendMode, Color, Fill, FontData, ImageBrushRef, StyleRef};

pub(crate) struct EngineScene<'p, S: PaintScene> {
    scene: &'p mut S,
    /// One entry per open layer: whether `push_layer` added a region clip
    /// layer under it, to pop with it.
    region_clips: Vec<bool>,
}

impl<'p, S: PaintScene> EngineScene<'p, S> {
    pub(crate) fn new(scene: &'p mut S) -> Self {
        Self {
            scene,
            region_clips: Vec::new(),
        }
    }
}

/// Stands in for "no filter region": far beyond any canvas, so the clip
/// anyrender_vello_cpu draws from it covers everything even when displaced.
const UNBOUNDED: Rect = Rect::new(-1e7, -1e7, 1e7, 1e7);

/// Horizontal skew that turns off anyrender_vello_cpu's glyph origin
/// snapping (`GlyphOriginSnap::new` returns None for any skew): it moves a
/// glyph at y = 100000 by 1/10000 px. Not vertical skew, which would also
/// turn off glifo's hinting.
const NO_SNAP_SKEW: f64 = 1e-9;

/// `transform` with [`NO_SNAP_SKEW`] when it is axis-aligned (the only
/// transforms anyrender_vello_cpu snaps).
fn unsnapped(transform: Affine) -> Affine {
    match transform.as_coeffs() {
        [a, 0.0, 0.0, d, e, f] => Affine::new([a, 0.0, NO_SNAP_SKEW, d, e, f]),
        _ => transform,
    }
}

/// Whether anyrender_vello_cpu 0.19 hands `filter` to vello_cpu (and so
/// clips it to its region): its first primitive is one vello_cpu implements
/// (`filters::convert_filter_effect` there).
fn painted_by_vello_cpu(filter: &Filter) -> bool {
    match filter.nodes().first().map(|node| &node.effect) {
        Some(FilterEffect::Flood(_) | FilterEffect::GaussianBlur(_) | FilterEffect::Offset(_)) => {
            true
        }
        Some(FilterEffect::DropShadow(shadow)) => shadow.std_deviation_x == shadow.std_deviation_y,
        _ => false,
    }
}

/// `filter` with an unbounded region, and the region it had.
fn split_region(filter: &Filter) -> (Filter, Rect) {
    let region = filter.nodes()[0].region;
    let mut unbounded = Filter::new(UNBOUNDED);
    for node in filter.nodes() {
        unbounded.add_node(FilterGraphNode {
            region: UNBOUNDED,
            ..node.clone()
        });
    }
    (unbounded, region)
}

/// `color` with each colour channel clamped to 0..=1 (alpha untouched).
pub(crate) fn clamp_to_srgb(color: Color) -> Color {
    let [r, g, b, a] = color.components;
    Color::new([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a])
}

impl<S: PaintScene> RenderContext for EngineScene<'_, S> {
    fn try_register_custom_resource(
        &mut self,
        resource: Box<dyn std::any::Any>,
    ) -> Result<ResourceId, RegisterResourceError> {
        self.scene.try_register_custom_resource(resource)
    }

    fn unregister_resource(&mut self, resource_id: ResourceId) {
        self.scene.unregister_resource(resource_id);
    }

    fn renderer_specific_context(&self) -> Option<Box<dyn std::any::Any>> {
        self.scene.renderer_specific_context()
    }
}

impl<S: PaintScene> PaintScene for EngineScene<'_, S> {
    fn reset(&mut self) {
        self.scene.reset();
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
        let (filter, region) = match filter {
            Some(filter) if painted_by_vello_cpu(&filter) => {
                let (unbounded, region) = split_region(&filter);
                (Some(Arc::new(unbounded)), Some(region))
            }
            filter => (filter, None),
        };
        if let Some(region) = region {
            // A path, not a rect: anyrender_vello_cpu clips to rects with
            // vello_cpu's non-isolated `push_clip_rect`.
            let region: BezPath = region.into_path(0.1);
            self.scene
                .push_clip_layer(Fill::NonZero, transform, &region);
        }
        self.region_clips.push(region.is_some());
        self.scene
            .push_layer(fill, blend, alpha, transform, clip, filter, backdrop_filter);
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &impl Shape) {
        self.region_clips.push(false);
        self.scene.push_clip_layer(fill, transform, clip);
    }

    fn push_mask_layer(
        &mut self,
        fill: Fill,
        mode: MaskMode,
        transform: Affine,
        clip: &impl Shape,
    ) {
        self.region_clips.push(false);
        self.scene.push_mask_layer(fill, mode, transform, clip);
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
        if self.region_clips.pop() == Some(true) {
            self.scene.pop_layer();
        }
    }

    fn stroke<'a>(
        &mut self,
        style: &Stroke,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
        anti_aliasing: AntiAliasing,
    ) {
        self.scene.stroke(
            style,
            transform,
            brush,
            brush_transform,
            shape,
            anti_aliasing,
        );
    }

    fn fill<'a>(
        &mut self,
        style: Fill,
        transform: Affine,
        brush: impl Into<PaintRef<'a>>,
        brush_transform: Option<Affine>,
        shape: &impl Shape,
        anti_aliasing: AntiAliasing,
    ) {
        self.scene.fill(
            style,
            transform,
            brush,
            brush_transform,
            shape,
            anti_aliasing,
        );
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
        anti_aliasing: AntiAliasing,
    ) {
        self.scene.draw_glyphs(
            font,
            font_size,
            hint,
            normalized_coords,
            embolden,
            style,
            brush,
            brush_alpha,
            unsnapped(transform),
            glyph_transform,
            glyphs,
            anti_aliasing,
        );
    }

    fn draw_box_shadow(
        &mut self,
        transform: Affine,
        box_shape: &NonUniformRoundedRect,
        offset: Vec2,
        spread: f64,
        std_dev: f64,
        color: Color,
        kind: BoxShadowKind,
    ) {
        self.scene.draw_box_shadow(
            transform,
            box_shape,
            offset,
            spread,
            std_dev,
            clamp_to_srgb(color),
            kind,
        );
    }

    fn draw_image(&mut self, image: ImageBrushRef, transform: Affine) {
        self.scene.draw_image(image, transform);
    }

    // `append_scene` keeps its default, which replays the recorded scene
    // through `self` so the guards above apply to it too.
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyrender::render_to_buffer;
    use anyrender_vello_cpu::VelloCpuImageRenderer;
    use peniko::Mix;

    const SIZE: u32 = 240;

    /// Red channel along the middle row (`along_x`) or column of a red box
    /// at 80..160 (user units, scaled by `scale`), painted in a layer with
    /// `effect` whose region is the box plus the effect's expansion, as
    /// blitz-paint builds it.
    fn red_line(effect: FilterEffect, scale: f64, along_x: bool) -> Vec<u8> {
        let bx = Rect::new(80.0 / scale, 80.0 / scale, 160.0 / scale, 160.0 / scale);
        let filter = Arc::new(Filter::single(effect, bx));
        let e = filter.expansion_rect();
        let clip = Rect::new(bx.x0 + e.x0, bx.y0 + e.y0, bx.x1 + e.x1, bx.y1 + e.y1);
        let transform = Affine::scale(scale);
        let buf = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                let mut scene = EngineScene::new(scene);
                scene.push_layer(
                    Fill::NonZero,
                    Mix::Normal,
                    1.0,
                    transform,
                    &clip,
                    Some(filter.clone()),
                    None,
                );
                scene.fill(
                    Fill::NonZero,
                    transform,
                    Color::from_rgba8(255, 0, 0, 255),
                    None,
                    &bx,
                    AntiAliasing::Enabled,
                );
                scene.pop_layer();
            },
            SIZE,
            SIZE,
        );
        (0..SIZE as usize)
            .map(|i| {
                let (x, y) = if along_x { (i, 120) } else { (120, i) };
                buf[(y * SIZE as usize + x) * 4]
            })
            .collect()
    }

    /// The halo of a blur centred on the box is the same on both sides.
    #[test]
    fn blur_halo_is_symmetric() {
        for (std_dev, scale) in [(3.0, 1.0), (4.0, 1.0), (4.0, 2.0), (1.0, 1.0)] {
            for along_x in [true, false] {
                let line = red_line(FilterEffect::blur(std_dev), scale, along_x);
                for d in 1..16 {
                    assert_eq!(
                        line[80 - d],
                        line[160 + d - 1],
                        "blur({std_dev}) at scale {scale}, {d}px out: {:?}",
                        &line[60..100]
                    );
                }
                assert!(line[79] > 20, "no halo: {:?}", &line[60..100]);
            }
        }
    }

    #[test]
    fn negative_offset_drop_shadow_keeps_its_halo() {
        let red = Color::from_rgba8(255, 0, 0, 255);
        let shadow = red_line(FilterEffect::drop_shadow(-12.0, 0.0, 4.0, red), 1.0, true);
        let blur = red_line(FilterEffect::blur(4.0), 1.0, true);
        // The shadow is the blurred box moved 12px left: same halo, from 68.
        assert!(shadow[66] > 20);
        for d in 1..16 {
            assert_eq!(
                shadow[68 - d],
                blur[80 - d],
                "{d}px out: {:?}",
                &shadow[40..90]
            );
        }
    }

    /// Two glyphs of the bundled Inter at 24px, origin `x` on a 40px
    /// baseline, drawn through [`EngineScene`] or straight to the backend.
    fn glyph_run(x: f64, through_engine_scene: bool) -> Vec<u8> {
        fn draw(scene: &mut impl PaintScene, font: &FontData, x: f64) {
            let glyphs = [
                Glyph {
                    id: 44,
                    x: 0.0,
                    y: 0.0,
                },
                Glyph {
                    id: 73,
                    x: 16.0,
                    y: 0.0,
                },
            ];
            scene.draw_glyphs(
                font,
                24.0,
                false,
                &[],
                Vec2::ZERO,
                Fill::NonZero,
                Color::from_rgba8(255, 255, 255, 255),
                1.0,
                Affine::translate((x, 40.0)),
                None,
                glyphs.into_iter(),
                AntiAliasing::Enabled,
            );
        }
        let font = FontData::new(peniko::Blob::new(Arc::new(crate::DEFAULT_FONT)), 0);
        render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                if through_engine_scene {
                    draw(&mut EngineScene::new(scene), &font, x);
                } else {
                    draw(scene, &font, x);
                }
            },
            64,
            64,
        )
    }

    /// At whole-pixel origins every platform's snap is the identity, so the
    /// skew that turns snapping off must change nothing there.
    #[test]
    fn unsnapped_glyphs_match_at_whole_pixels() {
        let direct = glyph_run(10.0, false);
        assert!(direct.iter().any(|&v| v > 0));
        assert_eq!(direct, glyph_run(10.0, true));
    }

    /// Off the pixel grid, glyphs move with their origin. Snapped, 10.1
    /// lands on 10 on every platform (Skia's nearest 1/4px, CoreGraphics'
    /// floor to 1/2px at 24ppem).
    #[test]
    fn unsnapped_glyphs_follow_fractional_origins() {
        assert_eq!(glyph_run(10.0, false), glyph_run(10.1, false));
        assert_ne!(glyph_run(10.0, true), glyph_run(10.1, true));
    }

    #[test]
    fn clamps_colour_channels_but_not_alpha() {
        let clamped = clamp_to_srgb(Color::new([-0.2, 0.8, 1.3, 0.5]));
        assert_eq!(clamped.components, [0.0, 0.8, 1.0, 0.5]);
    }
}
