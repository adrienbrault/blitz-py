//! Python bindings for the Blitz HTML/CSS rendering engine.
//!
//! Pipeline: html5ever (parse) -> Stylo (CSS cascade) -> Taffy (layout)
//! -> Parley (text shaping) -> blitz-paint -> vello_cpu (rasterize).
//! Fully headless, no GPU, no network, no JavaScript.

use std::sync::Arc;

use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use peniko::Fill;
use peniko::kurbo::Rect;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

/// A synchronous resource provider. Resolves `data:` URIs (and optionally
/// `file://` URLs) inline on the calling thread. All other schemes are
/// ignored, keeping rendering deterministic and network-free.
struct SyncProvider {
    allow_file_urls: bool,
}

impl NetProvider for SyncProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url;
        match url.scheme() {
            "data" => {
                let Ok(data_url) = data_url::DataUrl::process(url.as_str()) else {
                    return;
                };
                let Ok((bytes, _fragment)) = data_url.decode_to_vec() else {
                    return;
                };
                handler.bytes(url.to_string(), Bytes::from(bytes));
            }
            "file" if self.allow_file_urls => {
                if let Ok(contents) = std::fs::read(url.path()) {
                    handler.bytes(url.to_string(), Bytes::from(contents));
                }
            }
            _ => {}
        }
    }
}

fn parse_color_scheme(s: &str) -> PyResult<ColorScheme> {
    match s {
        "light" => Ok(ColorScheme::Light),
        "dark" => Ok(ColorScheme::Dark),
        _ => Err(PyValueError::new_err(format!(
            "color_scheme must be 'light' or 'dark', got '{s}'"
        ))),
    }
}

/// Parse a `#rgb`/`#rrggbb`/`#rrggbbaa` color, or None for transparent.
fn parse_background(s: Option<&str>) -> PyResult<Option<blitz_dom::util::Color>> {
    let Some(s) = s else { return Ok(None) };
    let hex = s.strip_prefix('#').unwrap_or(s);
    let parse =
        |h: &str| u8::from_str_radix(h, 16).map_err(|_| PyValueError::new_err("invalid color"));
    let (r, g, b, a) = match hex.len() {
        3 => {
            let d = |c: &str| parse(c).map(|v| v * 17);
            (d(&hex[0..1])?, d(&hex[1..2])?, d(&hex[2..3])?, 255)
        }
        6 => (parse(&hex[0..2])?, parse(&hex[2..4])?, parse(&hex[4..6])?, 255),
        8 => (
            parse(&hex[0..2])?,
            parse(&hex[2..4])?,
            parse(&hex[4..6])?,
            parse(&hex[6..8])?,
        ),
        _ => {
            return Err(PyValueError::new_err(format!(
                "background must be #rgb, #rrggbb or #rrggbbaa, got '{s}'"
            )));
        }
    };
    Ok(Some(blitz_dom::util::Color::from_rgba8(r, g, b, a)))
}

struct RenderResult {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

#[allow(clippy::too_many_arguments)]
fn render_impl(
    html: &str,
    width: u32,
    height: u32,
    scale: f32,
    color_scheme: ColorScheme,
    background: Option<blitz_dom::util::Color>,
    base_url: Option<String>,
    fonts: Vec<Vec<u8>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> RenderResult {
    let mut font_ctx = parley::FontContext::new();
    for font in fonts {
        let blob = parley::fontique::Blob::new(Arc::new(font));
        font_ctx.collection.register_fonts(blob, None);
    }
    if let Some(family) = default_font_family {
        use parley::fontique::{FallbackKey, GenericFamily, Script};
        let ids: Vec<_> = font_ctx
            .collection
            .family_id(&family)
            .into_iter()
            .collect();
        if !ids.is_empty() {
            for generic in [GenericFamily::Serif, GenericFamily::SansSerif, GenericFamily::Monospace] {
                font_ctx.collection.set_generic_families(generic, ids.iter().copied());
            }
            let latin: Script = "Latn".parse().expect("valid script tag");
            font_ctx
                .collection
                .set_fallbacks(FallbackKey::new(latin, None), ids.iter().copied());
        }
    }

    let physical_width = (width as f64 * scale as f64) as u32;
    let physical_height = (height as f64 * scale as f64) as u32;

    let mut document = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url,
            net_provider: Some(Arc::new(SyncProvider { allow_file_urls })),
            viewport: Some(Viewport::new(
                physical_width,
                physical_height,
                scale,
                color_scheme,
            )),
            font_ctx: Some(font_ctx),
            ..Default::default()
        },
    );

    // First resolve dispatches resource loads (delivered synchronously by
    // SyncProvider); second resolve restyles/relayouts with them applied.
    document.as_mut().resolve(0.0);
    document.as_mut().resolve(0.0);

    let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            if let Some(color) = background {
                scene.fill(
                    Fill::NonZero,
                    Default::default(),
                    color,
                    Default::default(),
                    &Rect::new(0.0, 0.0, physical_width as f64, physical_height as f64),
                );
            }
            paint_scene(
                scene,
                document.as_mut(),
                scale as f64,
                physical_width,
                physical_height,
                0,
                0,
            );
        },
        physical_width,
        physical_height,
    );

    RenderResult {
        rgba,
        width: physical_width,
        height: physical_height,
    }
}

fn encode_png(result: &RenderResult) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, result.width, result.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("failed to write PNG header");
    writer
        .write_image_data(&result.rgba)
        .expect("failed to write PNG data");
    writer.finish().expect("failed to finish PNG");
    out
}

/// Render an HTML string to a PNG image, returned as `bytes`.
#[pyfunction]
#[pyo3(signature = (html, *, width, height, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false))]
#[allow(clippy::too_many_arguments)]
fn render_png<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: u32,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let result = py.detach(|| {
        let result = render_impl(
            html,
            width,
            height,
            scale,
            color_scheme,
            background,
            base_url,
            fonts.unwrap_or_default(),
            default_font_family,
            allow_file_urls,
        );
        encode_png(&result)
    });
    Ok(PyBytes::new(py, &result))
}

/// Render an HTML string to raw RGBA pixels.
///
/// Returns `(width, height, bytes)` where `bytes` is `width * height * 4`
/// bytes of RGBA data — ready for `PIL.Image.frombytes("RGBA", (w, h), data)`.
#[pyfunction]
#[pyo3(signature = (html, *, width, height, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false))]
#[allow(clippy::too_many_arguments)]
fn render_rgba<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: u32,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let result = py.detach(|| {
        render_impl(
            html,
            width,
            height,
            scale,
            color_scheme,
            background,
            base_url,
            fonts.unwrap_or_default(),
            default_font_family,
            allow_file_urls,
        )
    });
    Ok((
        result.width,
        result.height,
        PyBytes::new(py, &result.rgba),
    ))
}

#[pymodule]
fn blitz_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(render_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_rgba, m)?)?;
    Ok(())
}
