//! Python bindings for the Blitz HTML/CSS rendering engine.
//!
//! Pipeline: html5ever (parse) -> Stylo (CSS cascade) -> Taffy (layout)
//! -> Parley (text shaping) -> blitz-paint -> vello_cpu (rasterize).
//! Fully headless, no GPU, no network, no JavaScript.

use std::sync::{Arc, Mutex, OnceLock};

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
                let Ok(path) = url.to_file_path() else { return };
                if let Ok(contents) = std::fs::read(path) {
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
    if !hex.is_ascii() {
        // Byte-offset slicing below requires ASCII; anything else is not a
        // valid hex color anyway.
        return Err(PyValueError::new_err(format!(
            "background must be #rgb, #rrggbb or #rrggbbaa, got '{s}'"
        )));
    }
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

/// Inter (SIL OFL 1.1, see assets/LICENSE-Inter.txt), bundled so that output
/// is identical across platforms and text renders on systems with no fonts
/// installed (e.g. Alpine containers).
static DEFAULT_FONT: &[u8] = include_bytes!("../assets/InterVariable.ttf");

/// Largest permitted physical output dimension. Guards against absurd
/// allocations (a 100_000² canvas is a 40GB buffer).
const MAX_DIM: u64 = 16_384;
/// Largest permitted physical pixel count (256MB of RGBA).
const MAX_PIXELS: u64 = 64_000_000;

fn validate(width: u32, height: u32, scale: f32) -> PyResult<()> {
    if width == 0 || height == 0 {
        return Err(PyValueError::new_err("width and height must be > 0"));
    }
    if !(scale.is_finite() && scale > 0.0) {
        return Err(PyValueError::new_err("scale must be a positive number"));
    }
    let pw = (width as f64 * scale as f64) as u64;
    let ph = (height as f64 * scale as f64) as u64;
    if pw == 0 || ph == 0 || pw > MAX_DIM || ph > MAX_DIM {
        return Err(PyValueError::new_err(format!(
            "physical output dimensions {pw}x{ph} outside supported range 1..={MAX_DIM}"
        )));
    }
    if pw * ph > MAX_PIXELS {
        return Err(PyValueError::new_err(format!(
            "physical output {pw}x{ph} exceeds {MAX_PIXELS} pixels"
        )));
    }
    Ok(())
}

fn set_default_ids(
    collection: &mut parley::fontique::Collection,
    ids: &[parley::fontique::FamilyId],
) {
    use parley::fontique::{FallbackKey, GenericFamily, Script};
    for generic in [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
    ] {
        collection.set_generic_families(generic, ids.iter().copied());
    }
    let latin: Script = "Latn".parse().expect("valid script tag");
    collection.set_fallbacks(FallbackKey::new(latin, None), ids.iter().copied());
}

fn set_default_family(collection: &mut parley::fontique::Collection, family: &str) -> bool {
    let ids: Vec<_> = collection.family_id(family).into_iter().collect();
    if ids.is_empty() {
        return false;
    }
    set_default_ids(collection, &ids);
    true
}

/// A pristine font collection: system fonts discovered once, bundled Inter
/// registered and set as the default. Cloned per render — constructing a
/// fresh collection each render both repeats the system-font scan and leaks
/// (~120KB/call in fontique 0.10's platform scan), which matters for
/// long-running processes.
fn base_collection() -> parley::fontique::Collection {
    use parley::fontique::{Blob, Collection, CollectionOptions};
    static BASE: OnceLock<Mutex<Collection>> = OnceLock::new();
    BASE.get_or_init(|| {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
            ..Default::default()
        });
        // Register under the explicit name "Inter" (the file's own family
        // name is "Inter Variable") and wire the defaults by returned id —
        // a name lookup here could silently match a system-installed Inter
        // or nothing at all.
        let registered = collection.register_fonts(
            Blob::new(Arc::new(DEFAULT_FONT)),
            Some(parley::fontique::FontInfoOverride {
                family_name: Some("Inter"),
                ..Default::default()
            }),
        );
        let ids: Vec<_> = registered.iter().map(|(id, _)| *id).collect();
        set_default_ids(&mut collection, &ids);
        Mutex::new(collection)
    })
    .lock()
    .expect("font collection lock poisoned")
    .clone()
}

#[allow(clippy::too_many_arguments)]
fn build_document(
    html: &str,
    width: u32,
    height: u32,
    scale: f32,
    color_scheme: ColorScheme,
    base_url: Option<String>,
    fonts: Vec<Vec<u8>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<HtmlDocument> {
    // Bundled Inter is the default for generic families and Latin fallback,
    // making output identical across platforms (and non-blank on systems
    // without fonts). Explicit CSS family names still resolve to system
    // fonts where available.
    let mut collection = base_collection();

    for font in fonts {
        let blob = parley::fontique::Blob::new(Arc::new(font));
        collection.register_fonts(blob, None);
    }
    if let Some(family) = &default_font_family {
        if !set_default_family(&mut collection, family) {
            return Err(PyValueError::new_err(format!(
                "default_font_family '{family}' not found among registered or system fonts"
            )));
        }
    }
    let font_ctx = parley::FontContext {
        collection,
        source_cache: Default::default(),
    };

    let physical_width = (width as f64 * scale as f64) as u32;
    let physical_height = (height as f64 * scale as f64) as u32;

    Ok(HtmlDocument::from_html(
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
    ))
}

fn paint_frame(
    document: &mut HtmlDocument,
    scale: f32,
    physical_width: u32,
    physical_height: u32,
    background: Option<blitz_dom::util::Color>,
) -> Vec<u8> {
    render_to_buffer::<VelloCpuImageRenderer, _>(
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
    )
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
) -> PyResult<RenderResult> {
    let mut document = build_document(
        html,
        width,
        height,
        scale,
        color_scheme,
        base_url,
        fonts,
        default_font_family,
        allow_file_urls,
    )?;
    let physical_width = (width as f64 * scale as f64) as u32;
    let physical_height = (height as f64 * scale as f64) as u32;

    // First resolve dispatches resource loads (delivered synchronously by
    // SyncProvider); second resolve restyles/relayouts with them applied.
    document.as_mut().resolve(0.0);
    document.as_mut().resolve(0.0);

    let rgba = paint_frame(&mut document, scale, physical_width, physical_height, background);

    Ok(RenderResult {
        rgba,
        width: physical_width,
        height: physical_height,
    })
}

const MAX_FRAMES: usize = 1000;

#[allow(clippy::too_many_arguments)]
fn render_frames_impl(
    html: &str,
    width: u32,
    height: u32,
    times: &[f64],
    scale: f32,
    color_scheme: ColorScheme,
    background: Option<blitz_dom::util::Color>,
    base_url: Option<String>,
    fonts: Vec<Vec<u8>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<(u32, u32, Vec<Vec<u8>>)> {
    let mut document = build_document(
        html,
        width,
        height,
        scale,
        color_scheme,
        base_url,
        fonts,
        default_font_family,
        allow_file_urls,
    )?;
    let physical_width = (width as f64 * scale as f64) as u32;
    let physical_height = (height as f64 * scale as f64) as u32;

    // Load resources at the first timestamp (see render_impl).
    document.as_mut().resolve(times[0]);
    document.as_mut().resolve(times[0]);

    let mut frames = Vec::with_capacity(times.len());
    for (i, &t) in times.iter().enumerate() {
        if i > 0 {
            document.as_mut().resolve(t);
        }
        frames.push(paint_frame(
            &mut document,
            scale,
            physical_width,
            physical_height,
            background,
        ));
    }
    Ok((physical_width, physical_height, frames))
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
    validate(width, height, scale)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let result = py.detach(|| -> PyResult<Vec<u8>> {
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
        )?;
        Ok(encode_png(&result))
    })?;
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
    validate(width, height, scale)?;
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
    })?;
    Ok((
        result.width,
        result.height,
        PyBytes::new(py, &result.rgba),
    ))
}

/// Render an HTML string at multiple animation timestamps.
///
/// `times` is a list of seconds on the document's animation clock; CSS
/// animations and transitions are evaluated at each instant, so a list like
/// `[i / 20 for i in range(40)]` yields 40 frames of a 2-second loop at
/// 20fps. Returns `(width, height, [rgba_bytes, ...])` — feed the frames to
/// Pillow to assemble a GIF:
///
/// ```python
/// w, h, frames = blitz_py.render_frames(html, width=240, height=240,
///                                       times=[i/20 for i in range(40)])
/// imgs = [Image.frombytes("RGBA", (w, h), f).convert("P") for f in frames]
/// imgs[0].save("out.gif", save_all=True, append_images=imgs[1:],
///              duration=50, loop=0)
/// ```
#[pyfunction]
#[pyo3(signature = (html, *, width, height, times, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false))]
#[allow(clippy::too_many_arguments)]
fn render_frames<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: u32,
    times: Vec<f64>,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<(u32, u32, Vec<Bound<'py, PyBytes>>)> {
    validate(width, height, scale)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    if times.is_empty() {
        return Err(PyValueError::new_err("times must not be empty"));
    }
    if times.len() > MAX_FRAMES {
        return Err(PyValueError::new_err(format!(
            "at most {MAX_FRAMES} frames per call (got {})",
            times.len()
        )));
    }
    if times.iter().any(|t| !t.is_finite() || *t < 0.0) {
        return Err(PyValueError::new_err(
            "times must be finite and non-negative",
        ));
    }
    let (w, h, frames) = py.detach(|| {
        render_frames_impl(
            html,
            width,
            height,
            &times,
            scale,
            color_scheme,
            background,
            base_url,
            fonts.unwrap_or_default(),
            default_font_family,
            allow_file_urls,
        )
    })?;
    let frames = frames.iter().map(|f| PyBytes::new(py, f)).collect();
    Ok((w, h, frames))
}

#[pymodule]
fn blitz_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(render_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(render_frames, m)?)?;
    Ok(())
}
