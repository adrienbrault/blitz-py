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

/// Parse a `#rgb`/`#rrggbb`/`#rrggbbaa` color into RGBA components.
fn parse_hex_rgba(s: &str) -> PyResult<(u8, u8, u8, u8)> {
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
    Ok((r, g, b, a))
}

/// Parse an optional color, `None` meaning transparent.
fn parse_background(s: Option<&str>) -> PyResult<Option<blitz_dom::util::Color>> {
    let Some(s) = s else { return Ok(None) };
    let (r, g, b, a) = parse_hex_rgba(s)?;
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

fn validate(width: u32, height: Option<u32>, scale: f32) -> PyResult<()> {
    if width == 0 || height == Some(0) {
        return Err(PyValueError::new_err("width and height must be > 0"));
    }
    if !(scale.is_finite() && scale > 0.0) {
        return Err(PyValueError::new_err("scale must be a positive number"));
    }
    let pw = (width as f64 * scale as f64) as u64;
    let ph = (height.unwrap_or(1) as f64 * scale as f64) as u64;
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

/// Compose the final HTML: user `css` and `css_vars` are appended in a
/// trailing `<style>` block so they win the cascade at equal specificity.
fn compose_html(
    html: &str,
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<String> {
    if css.is_none() && css_vars.is_none() {
        return Ok(html.to_string());
    }
    let mut style = String::new();
    if let Some(css) = css {
        style.push_str(css);
        style.push('\n');
    }
    if let Some(vars) = css_vars {
        let mut names: Vec<_> = vars.keys().collect();
        names.sort(); // deterministic output
        style.push_str(":root {\n");
        for name in names {
            if !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || name.is_empty()
            {
                return Err(PyValueError::new_err(format!(
                    "invalid css variable name '{name}'"
                )));
            }
            let value = &vars[name];
            // '<' could smuggle a premature `</style>` (HTML parsing does not
            // respect CSS string syntax); braces could escape the :root block.
            if value.contains(['{', '}', '<']) {
                return Err(PyValueError::new_err(format!(
                    "invalid css variable value for '{name}'"
                )));
            }
            let prefix = if name.starts_with("--") { "" } else { "--" };
            style.push_str(&format!("  {prefix}{name}: {value};\n"));
        }
        style.push_str("}\n");
    }
    Ok(format!("{html}<style>{style}</style>"))
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
fn base_mutex() -> &'static Mutex<parley::fontique::Collection> {
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
}

fn base_collection() -> parley::fontique::Collection {
    base_mutex()
        .lock()
        .expect("font collection lock poisoned")
        .clone()
}

/// Register fonts process-wide: every later render/measure sees them without
/// passing bytes per call. Returns the family names registered. Optionally
/// makes one family the default (generic families + Latin fallback).
///
/// Global mutable state: for deterministic output, register at startup,
/// before rendering.
#[pyfunction]
#[pyo3(signature = (fonts, *, default_family=None))]
fn register_fonts(
    py: Python<'_>,
    fonts: Vec<Vec<u8>>,
    default_family: Option<String>,
) -> PyResult<Vec<String>> {
    py.detach(move || {
        let mut base = base_mutex()
            .lock()
            .map_err(|_| PyValueError::new_err("font collection lock poisoned"))?;
        let mut names = Vec::new();
        for font in fonts {
            let registered =
                base.register_fonts(parley::fontique::Blob::new(Arc::new(font)), None);
            for (id, _) in registered {
                if let Some(name) = base.family_name(id) {
                    names.push(name.to_string());
                }
            }
        }
        if let Some(family) = default_family {
            if !set_default_family(&mut base, &family) {
                return Err(PyValueError::new_err(format!(
                    "default_family '{family}' not found among registered or system fonts"
                )));
            }
        }
        Ok(names)
    })
}

/// Build a Parley layout for the text-utility functions.
fn text_layout(
    text: &str,
    family: &str,
    font_size: f32,
    font_weight: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    fonts: &[Vec<u8>],
) -> parley::Layout<[u8; 4]> {
    let mut collection = base_collection();
    for font in fonts {
        collection.register_fonts(parley::fontique::Blob::new(Arc::new(font.clone())), None);
    }
    let mut font_ctx = parley::FontContext {
        collection,
        source_cache: Default::default(),
    };
    let mut layout_ctx: parley::LayoutContext<[u8; 4]> = parley::LayoutContext::new();
    let mut builder = layout_ctx.ranged_builder(&mut font_ctx, text, 1.0, true);
    builder.push_default(parley::StyleProperty::FontFamily(
        parley::style::FontFamily::Source(std::borrow::Cow::Borrowed(family)),
    ));
    builder.push_default(parley::StyleProperty::FontSize(font_size));
    builder.push_default(parley::StyleProperty::FontWeight(parley::FontWeight::new(
        font_weight,
    )));
    builder.push_default(parley::StyleProperty::LetterSpacing(letter_spacing));
    let mut layout = builder.build(text);
    layout.break_all_lines(max_width);
    layout
}

fn validate_text_style(font_size: f32, font_weight: f32) -> PyResult<()> {
    if !(font_size.is_finite() && font_size > 0.0) {
        return Err(PyValueError::new_err("font_size must be a positive number"));
    }
    if !(1.0..=1000.0).contains(&font_weight) {
        return Err(PyValueError::new_err("font_weight must be in 1..=1000"));
    }
    Ok(())
}

/// Per-line metrics of wrapped text: list of `(width, height)` per line.
#[pyfunction]
#[pyo3(signature = (text, *, font_size, max_width=None, font_family=None, font_weight=400.0, letter_spacing=0.0, fonts=None))]
#[allow(clippy::too_many_arguments)]
fn measure_text_lines(
    py: Python<'_>,
    text: &str,
    font_size: f32,
    max_width: Option<f32>,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<Vec<(f64, f64)>> {
    validate_text_style(font_size, font_weight)?;
    let family = font_family.unwrap_or("Inter").to_string();
    let text = text.to_string();
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let layout = text_layout(
            &text,
            &family,
            font_size,
            font_weight,
            letter_spacing,
            max_width,
            &fonts,
        );
        Ok(layout
            .lines()
            .map(|line| {
                let m = line.metrics();
                (m.advance as f64, m.line_height as f64)
            })
            .collect())
    })
}

/// Truncate `text` with an ellipsis so it fits in `max_width` on one line.
/// Uses the engine's own shaper, so the result is exact for rendering.
#[pyfunction]
#[pyo3(signature = (text, *, max_width, font_size, font_family=None, font_weight=400.0, letter_spacing=0.0, ellipsis="…", fonts=None))]
#[allow(clippy::too_many_arguments)]
fn ellipsize(
    py: Python<'_>,
    text: &str,
    max_width: f32,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    ellipsis: &str,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<String> {
    validate_text_style(font_size, font_weight)?;
    let family = font_family.unwrap_or("Inter").to_string();
    let (text, ellipsis) = (text.to_string(), ellipsis.to_string());
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let width = |s: &str| {
            text_layout(s, &family, font_size, font_weight, letter_spacing, None, &fonts).width()
        };
        if width(&text) <= max_width {
            return Ok(text);
        }
        let chars: Vec<char> = text.chars().collect();
        // Largest prefix such that prefix+ellipsis fits.
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + &ellipsis;
            if width(candidate.trim_end()) <= max_width {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let prefix: String = chars[..lo].iter().collect();
        Ok(prefix.trim_end().to_string() + &ellipsis)
    })
}

/// Truncate `text` with an ellipsis so it wraps to at most `max_lines` lines
/// of `max_width` (a `-webkit-line-clamp` equivalent, computed up front).
#[pyfunction]
#[pyo3(signature = (text, *, max_width, max_lines, font_size, font_family=None, font_weight=400.0, letter_spacing=0.0, ellipsis="…", fonts=None))]
#[allow(clippy::too_many_arguments)]
fn line_clamp(
    py: Python<'_>,
    text: &str,
    max_width: f32,
    max_lines: usize,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    ellipsis: &str,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<String> {
    validate_text_style(font_size, font_weight)?;
    if max_lines == 0 {
        return Err(PyValueError::new_err("max_lines must be > 0"));
    }
    let family = font_family.unwrap_or("Inter").to_string();
    let (text, ellipsis) = (text.to_string(), ellipsis.to_string());
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let line_count = |s: &str| {
            text_layout(
                s,
                &family,
                font_size,
                font_weight,
                letter_spacing,
                Some(max_width),
                &fonts,
            )
            .lines()
            .count()
        };
        if line_count(&text) <= max_lines {
            return Ok(text);
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0usize, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String =
                chars[..mid].iter().collect::<String>().trim_end().to_string() + &ellipsis;
            if line_count(&candidate) <= max_lines {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let prefix: String = chars[..lo].iter().collect();
        Ok(prefix.trim_end().to_string() + &ellipsis)
    })
}

/// Largest font size in `[min_size, max_size]` at which `text` fits
/// `max_width` (single line), or — with `wrap=True` and `max_height` — fits
/// the box when wrapped. The SwiftUI `minimumScaleFactor` pattern.
#[pyfunction]
#[pyo3(signature = (text, *, max_width, max_size, min_size=6.0, max_height=None, wrap=false, font_family=None, font_weight=400.0, letter_spacing=0.0, fonts=None))]
#[allow(clippy::too_many_arguments)]
fn fit_font_size(
    py: Python<'_>,
    text: &str,
    max_width: f32,
    max_size: f32,
    min_size: f32,
    max_height: Option<f32>,
    wrap: bool,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<f64> {
    validate_text_style(max_size, font_weight)?;
    if !(min_size > 0.0 && min_size <= max_size) {
        return Err(PyValueError::new_err("need 0 < min_size <= max_size"));
    }
    if wrap && max_height.is_none() {
        return Err(PyValueError::new_err("wrap=True requires max_height"));
    }
    let family = font_family.unwrap_or("Inter").to_string();
    let text = text.to_string();
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let fits = |size: f32| {
            let layout = text_layout(
                &text,
                &family,
                size,
                font_weight,
                letter_spacing,
                if wrap { Some(max_width) } else { None },
                &fonts,
            );
            let width_ok = layout.width() <= max_width;
            let height_ok = max_height.is_none_or(|h| layout.height() <= h);
            width_ok && height_ok
        };
        let (mut lo, mut hi) = (min_size, max_size);
        if fits(hi) {
            return Ok(hi as f64);
        }
        // Binary search to 0.25px granularity.
        while hi - lo > 0.25 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Ok(lo as f64)
    })
}

/// Break `text` into balanced lines fitting `max_width` (the
/// `text-wrap: balance` behavior): uses the minimum line count, then evens
/// the lines out. Returns the lines; join with `<br>` in HTML.
#[pyfunction]
#[pyo3(signature = (text, *, max_width, font_size, font_family=None, font_weight=400.0, letter_spacing=0.0, fonts=None))]
#[allow(clippy::too_many_arguments)]
fn wrap_balanced(
    py: Python<'_>,
    text: &str,
    max_width: f32,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<Vec<String>> {
    validate_text_style(font_size, font_weight)?;
    let family = font_family.unwrap_or("Inter").to_string();
    let text = text.to_string();
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return Ok(vec![]);
        }
        let measure = |s: &str| {
            text_layout(s, &family, font_size, font_weight, letter_spacing, None, &fonts).width()
        };
        // Greedy break at a given width; None if any single word overflows.
        let greedy = |limit: f32| -> Option<Vec<String>> {
            let mut lines: Vec<String> = Vec::new();
            let mut current = String::new();
            for word in &words {
                if measure(word) > limit {
                    return None;
                }
                let candidate = if current.is_empty() {
                    (*word).to_string()
                } else {
                    format!("{current} {word}")
                };
                if measure(&candidate) <= limit {
                    current = candidate;
                } else {
                    lines.push(std::mem::take(&mut current));
                    current = (*word).to_string();
                }
            }
            lines.push(current);
            Some(lines)
        };
        let Some(base) = greedy(max_width) else {
            // A single word wider than the box: return greedy-per-word.
            return Ok(words.iter().map(|w| w.to_string()).collect());
        };
        let target_lines = base.len();
        if target_lines == 1 {
            return Ok(base);
        }
        // Shrink the width until the line count would grow; the narrowest
        // width preserving the count gives the most balanced fill.
        let longest_word = words.iter().map(|w| measure(w)).fold(0.0f32, f32::max);
        let (mut lo, mut hi) = (longest_word, max_width);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.0;
            match greedy(mid) {
                Some(lines) if lines.len() <= target_lines => hi = mid,
                _ => lo = mid,
            }
        }
        Ok(greedy(hi).unwrap_or(base))
    })
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
            // Required for set_inner_html (Template.set_html); from_html does
            // not install one by itself.
            html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider)),
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
    height: Option<u32>,
    scale: f32,
    color_scheme: ColorScheme,
    background: Option<blitz_dom::util::Color>,
    base_url: Option<String>,
    fonts: Vec<Vec<u8>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
) -> PyResult<RenderResult> {
    // With height=None, lay out against a provisional viewport, then size the
    // canvas to the content.
    let provisional_height = height.unwrap_or(800);
    let mut document = build_document(
        html,
        width,
        provisional_height,
        scale,
        color_scheme,
        base_url,
        fonts,
        default_font_family,
        allow_file_urls,
    )?;
    let physical_width = (width as f64 * scale as f64) as u32;

    // First resolve dispatches resource loads (delivered synchronously by
    // SyncProvider); second resolve restyles/relayouts with them applied.
    document.as_mut().resolve(0.0);
    document.as_mut().resolve(0.0);

    let physical_height = match height {
        Some(h) => (h as f64 * scale as f64) as u32,
        None => {
            let content_css_px = document.as_ref().root_element().final_layout.size.height;
            let ph = ((content_css_px as f64) * scale as f64).ceil().max(1.0) as u64;
            if ph > MAX_DIM || (physical_width as u64) * ph > MAX_PIXELS {
                return Err(PyValueError::new_err(format!(
                    "auto height {ph}px exceeds output limits"
                )));
            }
            let ph = ph as u32;
            // Re-lay out with the real viewport so vh units and % heights
            // resolve against the final canvas.
            document.as_mut().set_viewport(Viewport::new(
                physical_width,
                ph,
                scale,
                color_scheme,
            ));
            document.as_mut().resolve(0.0);
            ph
        }
    };

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

fn encode_jpeg(result: &RenderResult, quality: u8) -> PyResult<Vec<u8>> {
    let mut out = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder
        .encode(
            &result.rgba,
            result.width as u16,
            result.height as u16,
            jpeg_encoder::ColorType::Rgba,
        )
        .map_err(|e| PyValueError::new_err(format!("jpeg encoding failed: {e}")))?;
    Ok(out)
}

fn validate_quality(quality: u8) -> PyResult<()> {
    if !(1..=100).contains(&quality) {
        return Err(PyValueError::new_err("quality must be in 1..=100"));
    }
    Ok(())
}

/// Encode RGBA frames as an infinitely-looping GIF.
///
/// One global palette is quantized (NeuQuant) from pixels sampled across all
/// frames; no dithering (dither noise defeats LZW compression on UI-style
/// content); frames after the first are cropped to the bounding box of
/// changed pixels. Per-frame delays derive from consecutive `times` deltas.
fn encode_gif(
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    times: &[f64],
    colors: u16,
) -> PyResult<Vec<u8>> {
    use std::collections::HashMap;

    let (w, h) = (width as usize, height as usize);
    let npx = w * h;

    // Sample up to ~64k pixels evenly across all frames for the palette.
    let total = npx * frames.len();
    let step = (total / 65_536).max(1);
    let mut samples = Vec::with_capacity((total / step + 1) * 4);
    for i in (0..total).step_by(step) {
        let (f, p) = (i / npx, i % npx);
        let px = &frames[f][p * 4..p * 4 + 4];
        samples.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    // Cap at 255 so the reserved transparent slot below still fits in a u8.
    let nq = color_quant::NeuQuant::new(10, (colors as usize).min(255), &samples);
    let mut palette_rgb: Vec<u8> = nq
        .color_map_rgba()
        .chunks_exact(4)
        .flat_map(|c| [c[0], c[1], c[2]])
        .collect();
    // Reserve one extra palette slot as the inter-frame "unchanged" marker:
    // pixels equal to the previous frame become this transparent index with
    // disposal=Keep, which LZW compresses into long runs (the trick that
    // makes UI animation GIFs small).
    let transparent_idx = (palette_rgb.len() / 3) as u8;
    palette_rgb.extend_from_slice(&[0, 0, 0]);

    // Index frames through a color cache (UI renders have few unique colors).
    let mut cache: HashMap<[u8; 3], u8> = HashMap::new();
    let indexed: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| {
            frame
                .chunks_exact(4)
                .map(|px| {
                    let key = [px[0], px[1], px[2]];
                    *cache
                        .entry(key)
                        .or_insert_with(|| nq.index_of(&[px[0], px[1], px[2], 255]) as u8)
                })
                .collect()
        })
        .collect();

    // Delays in centiseconds from time deltas (loop-closing last delay).
    let delay_cs = |dt: f64| ((dt * 100.0).round() as i64).clamp(2, u16::MAX as i64) as u16;
    let delays: Vec<u16> = (0..frames.len())
        .map(|i| {
            if times.len() < 2 {
                10
            } else if i + 1 < times.len() {
                delay_cs(times[i + 1] - times[i])
            } else {
                delay_cs(times[times.len() - 1] - times[times.len() - 2])
            }
        })
        .collect();

    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, width as u16, height as u16, &palette_rgb)
            .map_err(|e| PyValueError::new_err(format!("gif encoding failed: {e}")))?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| PyValueError::new_err(format!("gif encoding failed: {e}")))?;

        for (i, idx) in indexed.iter().enumerate() {
            // Crop to the changed region (full frame for the first).
            let (mut x0, mut y0, mut x1, mut y1) = (0usize, 0usize, w, h);
            if i > 0 {
                let prev = &indexed[i - 1];
                let changed_rows: Vec<usize> = (0..h)
                    .filter(|&y| idx[y * w..(y + 1) * w] != prev[y * w..(y + 1) * w])
                    .collect();
                match (changed_rows.first(), changed_rows.last()) {
                    (Some(&top), Some(&bottom)) => {
                        y0 = top;
                        y1 = bottom + 1;
                        x0 = w;
                        x1 = 0;
                        for &y in &changed_rows {
                            for x in 0..w {
                                if idx[y * w + x] != prev[y * w + x] {
                                    x0 = x0.min(x);
                                    x1 = x1.max(x + 1);
                                }
                            }
                        }
                    }
                    _ => {
                        // Identical frame: emit a 1x1 patch to carry the delay.
                        (x0, y0, x1, y1) = (0, 0, 1, 1);
                    }
                }
            }
            let (fw, fh) = (x1 - x0, y1 - y0);
            let mut buffer = Vec::with_capacity(fw * fh);
            for y in y0..y1 {
                if i == 0 {
                    buffer.extend_from_slice(&idx[y * w + x0..y * w + x1]);
                } else {
                    let prev = &indexed[i - 1];
                    for x in x0..x1 {
                        let p = y * w + x;
                        buffer.push(if idx[p] == prev[p] {
                            transparent_idx
                        } else {
                            idx[p]
                        });
                    }
                }
            }
            let frame = gif::Frame {
                delay: delays[i],
                top: y0 as u16,
                left: x0 as u16,
                width: fw as u16,
                height: fh as u16,
                buffer: std::borrow::Cow::Owned(buffer),
                transparent: if i == 0 { None } else { Some(transparent_idx) },
                dispose: gif::DisposalMethod::Keep,
                ..Default::default()
            };
            encoder
                .write_frame(&frame)
                .map_err(|e| PyValueError::new_err(format!("gif encoding failed: {e}")))?;
        }
    }
    Ok(out)
}

fn validate_gif_args(times: &[f64], colors: u16) -> PyResult<()> {
    if !(2..=256).contains(&colors) {
        return Err(PyValueError::new_err("colors must be in 2..=256"));
    }
    validate_times(times)
}

/// Run `f` on a thread with a large explicit stack. Style and layout
/// traversals recurse per DOM-nesting level; Windows threads default to
/// ~1MB of stack (vs 8MB on Linux/macOS), which deeply nested documents
/// overflow — crashing the whole process. 64MB is reserved lazily, so the
/// cost is only the ~tens of µs thread spawn.
fn with_render_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("blitz-py-render".into())
            .stack_size(64 << 20)
            .spawn_scoped(scope, f)
            .expect("failed to spawn render thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
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
///
/// With `height=None` the canvas height is sized to the laid-out content.
#[pyfunction]
#[pyo3(signature = (html, *, width, height=None, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
#[allow(clippy::too_many_arguments)]
fn render_png<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: Option<u32>,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<Bound<'py, PyBytes>> {
    validate(width, height, scale)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let html = compose_html(html, css, css_vars)?;
    let result = py.detach(|| -> PyResult<Vec<u8>> {
        with_render_stack(|| {
        let result = render_impl(
            &html,
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
        })
    })?;
    Ok(PyBytes::new(py, &result))
}

/// Render an HTML string to raw RGBA pixels.
///
/// Returns `(width, height, bytes)` where `bytes` is `width * height * 4`
/// bytes of RGBA data — ready for `PIL.Image.frombytes("RGBA", (w, h), data)`.
/// With `height=None` the canvas height is sized to the laid-out content.
#[pyfunction]
#[pyo3(signature = (html, *, width, height=None, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
#[allow(clippy::too_many_arguments)]
fn render_rgba<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: Option<u32>,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    validate(width, height, scale)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let html = compose_html(html, css, css_vars)?;
    let result = py.detach(|| {
        with_render_stack(|| {
        render_impl(
            &html,
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
        })
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
#[pyo3(signature = (html, *, width, height, times, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
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
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<(u32, u32, Vec<Bound<'py, PyBytes>>)> {
    validate(width, Some(height), scale)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let html = compose_html(html, css, css_vars)?;
    validate_times(&times)?;
    let (w, h, frames) = py.detach(|| {
        with_render_stack(|| {
        render_frames_impl(
            &html,
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
        })
    })?;
    let frames = frames.iter().map(|f| PyBytes::new(py, f)).collect();
    Ok((w, h, frames))
}

/// Render an HTML string to a JPEG image, returned as `bytes`.
///
/// JPEG has no alpha channel — use an opaque `background` (the default).
/// With `height=None` the canvas height is sized to the laid-out content.
#[pyfunction]
#[pyo3(signature = (html, *, width, height=None, quality=90, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
#[allow(clippy::too_many_arguments)]
fn render_jpeg<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: Option<u32>,
    quality: u8,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<Bound<'py, PyBytes>> {
    validate(width, height, scale)?;
    validate_quality(quality)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let html = compose_html(html, css, css_vars)?;
    let result = py.detach(|| -> PyResult<Vec<u8>> {
        with_render_stack(|| {
            let result = render_impl(
                &html,
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
            encode_jpeg(&result, quality)
        })
    })?;
    Ok(PyBytes::new(py, &result))
}

/// Render CSS-animation frames and encode them as an infinitely-looping GIF.
///
/// Frame delays follow the spacing of `times`. One shared `colors`-entry
/// palette, no dithering, delta-encoded frames — tuned for UI-style content.
#[pyfunction]
#[pyo3(signature = (html, *, width, height, times, colors=64, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
#[allow(clippy::too_many_arguments)]
fn render_gif<'py>(
    py: Python<'py>,
    html: &str,
    width: u32,
    height: u32,
    times: Vec<f64>,
    colors: u16,
    scale: f32,
    color_scheme: &str,
    background: Option<&str>,
    base_url: Option<String>,
    fonts: Option<Vec<Vec<u8>>>,
    default_font_family: Option<String>,
    allow_file_urls: bool,
    css: Option<&str>,
    css_vars: Option<std::collections::HashMap<String, String>>,
) -> PyResult<Bound<'py, PyBytes>> {
    validate(width, Some(height), scale)?;
    validate_gif_args(&times, colors)?;
    let color_scheme = parse_color_scheme(color_scheme)?;
    let background = parse_background(background)?;
    let html = compose_html(html, css, css_vars)?;
    let gif = py.detach(|| -> PyResult<Vec<u8>> {
        with_render_stack(|| {
            let (w, h, frames) = render_frames_impl(
                &html,
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
            )?;
            encode_gif(w, h, &frames, &times, colors)
        })
    })?;
    Ok(PyBytes::new(py, &gif))
}

/// Measure text with the same shaping engine and font collection used for
/// rendering (Parley + bundled Inter + registered fonts) — one source of
/// truth for layout math done in Python (ellipsis, fitting, wrapping).
///
/// Returns `(width, height)` in CSS pixels. `font_family` accepts a CSS
/// font-family list (e.g. `"Inter, sans-serif"`). With `max_width` the text
/// wraps and `height` reflects the wrapped line count.
#[pyfunction]
#[pyo3(signature = (text, *, font_size, font_family=None, font_weight=400.0, letter_spacing=0.0, max_width=None, fonts=None))]
#[allow(clippy::too_many_arguments)]
fn measure_text(
    py: Python<'_>,
    text: &str,
    font_size: f32,
    font_family: Option<&str>,
    font_weight: f32,
    letter_spacing: f32,
    max_width: Option<f32>,
    fonts: Option<Vec<Vec<u8>>>,
) -> PyResult<(f64, f64)> {
    if !(font_size.is_finite() && font_size > 0.0) {
        return Err(PyValueError::new_err("font_size must be a positive number"));
    }
    if !(1.0..=1000.0).contains(&font_weight) {
        return Err(PyValueError::new_err("font_weight must be in 1..=1000"));
    }
    let family = font_family.unwrap_or("Inter").to_string();
    let text = text.to_string();
    let fonts = fonts.unwrap_or_default();
    py.detach(move || {
        let mut collection = base_collection();
        for font in fonts {
            collection.register_fonts(parley::fontique::Blob::new(Arc::new(font)), None);
        }
        let mut font_ctx = parley::FontContext {
            collection,
            source_cache: Default::default(),
        };
        let mut layout_ctx: parley::LayoutContext<[u8; 4]> = parley::LayoutContext::new();
        let mut builder = layout_ctx.ranged_builder(&mut font_ctx, &text, 1.0, true);
        builder.push_default(parley::StyleProperty::FontFamily(
            parley::style::FontFamily::Source(std::borrow::Cow::Borrowed(family.as_str())),
        ));
        builder.push_default(parley::StyleProperty::FontSize(font_size));
        builder.push_default(parley::StyleProperty::FontWeight(
            parley::FontWeight::new(font_weight),
        ));
        builder.push_default(parley::StyleProperty::LetterSpacing(letter_spacing));
        let mut layout = builder.build(&text);
        layout.break_all_lines(max_width);
        Ok((layout.width() as f64, layout.height() as f64))
    })
}

enum TemplateCmd {
    SetText {
        id: String,
        text: String,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    SetStyle {
        id: String,
        name: String,
        value: String,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    SetAttr {
        id: String,
        name: String,
        value: String,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    SetHtml {
        id: String,
        html: String,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    UpdateTexts {
        updates: Vec<(String, String)>,
        reply: std::sync::mpsc::Sender<Result<(), String>>,
    },
    Render {
        time: f64,
        reply: std::sync::mpsc::Sender<RenderResult>,
    },
    RenderFrames {
        times: Vec<f64>,
        reply: std::sync::mpsc::Sender<(u32, u32, Vec<Vec<u8>>)>,
    },
    GetBox {
        id: String,
        reply: std::sync::mpsc::Sender<Result<(f64, f64, f64, f64), String>>,
    },
    Boxes {
        reply: std::sync::mpsc::Sender<Vec<(String, (f64, f64, f64, f64))>>,
    },
}

/// Walk the tree collecting `id` attributes and, per element, its single
/// text-node child (if it has exactly one child and that child is text).
fn build_id_map(
    doc: &blitz_dom::BaseDocument,
) -> std::collections::HashMap<String, (usize, Option<usize>)> {
    let mut map = std::collections::HashMap::new();
    let mut stack = vec![doc.root_element().id];
    while let Some(node_id) = stack.pop() {
        let Some(node) = doc.get_node(node_id) else { continue };
        if let Some(el) = node.element_data() {
            if let Some(id_attr) = el.attr(blitz_dom::local_name!("id")) {
                let text_child = match node.children.as_slice() {
                    [only] => doc
                        .get_node(*only)
                        .filter(|c| c.text_data().is_some())
                        .map(|c| c.id),
                    _ => None,
                };
                map.insert(id_attr.to_string(), (node_id, text_child));
            }
        }
        stack.extend(node.children.iter().copied());
    }
    map
}

/// Insert or replace `name: value` in an inline style declaration list.
/// Splits on `;` only outside parentheses (data: URLs contain semicolons).
fn upsert_style(current: &str, name: &str, value: &str) -> String {
    let mut decls: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in current.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                decls.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    decls.push(cur);
    decls.retain(|d| {
        let prop = d.split(':').next().unwrap_or("").trim();
        !d.trim().is_empty() && !prop.eq_ignore_ascii_case(name)
    });
    decls.push(format!("{name}: {value}"));
    decls.join("; ")
}

struct TemplateWorker {
    document: HtmlDocument,
    scale: f32,
    physical_width: u32,
    physical_height: u32,
    background: Option<blitz_dom::util::Color>,
    id_map: std::collections::HashMap<String, (usize, Option<usize>)>,
}

impl TemplateWorker {
    fn lookup(&self, id: &str) -> Result<(usize, Option<usize>), String> {
        self.id_map
            .get(id)
            .copied()
            .ok_or_else(|| format!("no element with id '{id}'"))
    }

    /// Absolute laid-out rect of a node in CSS pixels (post-resolve).
    fn abs_box(&self, node_id: usize) -> (f64, f64, f64, f64) {
        let doc = self.document.as_ref();
        let Some(node) = doc.get_node(node_id) else {
            return (0.0, 0.0, 0.0, 0.0);
        };
        let (mut x, mut y) = (
            node.final_layout.location.x as f64,
            node.final_layout.location.y as f64,
        );
        let (w, h) = (
            node.final_layout.size.width as f64,
            node.final_layout.size.height as f64,
        );
        let mut current = node.parent;
        while let Some(parent_id) = current {
            let Some(parent) = doc.get_node(parent_id) else { break };
            x += parent.final_layout.location.x as f64;
            y += parent.final_layout.location.y as f64;
            current = parent.parent;
        }
        (x, y, w, h)
    }

    fn run(mut self, rx: std::sync::mpsc::Receiver<TemplateCmd>) {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                TemplateCmd::SetText { id, text, reply } => {
                    let result = self.lookup(&id).and_then(|(_, text_node)| {
                        let text_node = text_node.ok_or_else(|| {
                            format!("element '{id}' does not contain exactly one text node")
                        })?;
                        blitz_dom::DocumentMutator::new(self.document.as_mut())
                            .set_node_text(text_node, &text);
                        Ok(())
                    });
                    let _ = reply.send(result);
                }
                TemplateCmd::SetStyle { id, name, value, reply } => {
                    // Rewrite the `style` attribute rather than using
                    // `set_style_property`: property mutation misses layout
                    // invalidation in blitz 0.3.0-beta.1 (fixed upstream
                    // post-release), while attribute mutation invalidates
                    // correctly.
                    let result = self.lookup(&id).map(|(node_id, _)| {
                        let current = self
                            .document
                            .as_ref()
                            .get_node(node_id)
                            .and_then(|n| n.element_data())
                            .and_then(|el| el.attr(blitz_dom::local_name!("style")))
                            .unwrap_or_default()
                            .to_string();
                        let style = upsert_style(&current, &name, &value);
                        let qual = blitz_dom::QualName::new(
                            None,
                            blitz_dom::ns!(),
                            blitz_dom::local_name!("style"),
                        );
                        blitz_dom::DocumentMutator::new(self.document.as_mut())
                            .set_attribute(node_id, qual, &style);
                    });
                    let _ = reply.send(result);
                }
                TemplateCmd::SetAttr { id, name, value, reply } => {
                    let result = self.lookup(&id).map(|(node_id, _)| {
                        let qual =
                            blitz_dom::QualName::new(None, blitz_dom::ns!(), name.as_str().into());
                        blitz_dom::DocumentMutator::new(self.document.as_mut())
                            .set_attribute(node_id, qual, &value);
                    });
                    let _ = reply.send(result);
                }
                TemplateCmd::SetHtml { id, html, reply } => {
                    let result = self.lookup(&id).map(|(node_id, _)| {
                        blitz_dom::DocumentMutator::new(self.document.as_mut())
                            .set_inner_html(node_id, &html);
                        // The fragment may add or remove elements with ids
                        // (including sole-text-child status of this element).
                        self.id_map = build_id_map(self.document.as_ref());
                    });
                    let _ = reply.send(result);
                }
                TemplateCmd::UpdateTexts { updates, reply } => {
                    // Validate every id before applying any, so a failed
                    // batch leaves the document untouched.
                    let mut nodes = Vec::with_capacity(updates.len());
                    let mut error = None;
                    for (id, text) in &updates {
                        match self.lookup(id) {
                            Ok((_, Some(text_node))) => nodes.push((text_node, text)),
                            Ok((_, None)) => {
                                error = Some(format!(
                                    "element '{id}' does not contain exactly one text node"
                                ));
                                break;
                            }
                            Err(e) => {
                                error = Some(e);
                                break;
                            }
                        }
                    }
                    let result = match error {
                        Some(e) => Err(e),
                        None => {
                            let mut mutator =
                                blitz_dom::DocumentMutator::new(self.document.as_mut());
                            for (text_node, text) in nodes {
                                mutator.set_node_text(text_node, text);
                            }
                            Ok(())
                        }
                    };
                    let _ = reply.send(result);
                }
                TemplateCmd::Render { time, reply } => {
                    self.document.as_mut().resolve(time);
                    let rgba = paint_frame(
                        &mut self.document,
                        self.scale,
                        self.physical_width,
                        self.physical_height,
                        self.background,
                    );
                    let _ = reply.send(RenderResult {
                        rgba,
                        width: self.physical_width,
                        height: self.physical_height,
                    });
                }
                TemplateCmd::RenderFrames { times, reply } => {
                    let mut frames = Vec::with_capacity(times.len());
                    for &t in &times {
                        self.document.as_mut().resolve(t);
                        frames.push(paint_frame(
                            &mut self.document,
                            self.scale,
                            self.physical_width,
                            self.physical_height,
                            self.background,
                        ));
                    }
                    let _ = reply.send((self.physical_width, self.physical_height, frames));
                }
                TemplateCmd::GetBox { id, reply } => {
                    self.document.as_mut().resolve(0.0);
                    let result = self.lookup(&id).map(|(node_id, _)| self.abs_box(node_id));
                    let _ = reply.send(result);
                }
                TemplateCmd::Boxes { reply } => {
                    self.document.as_mut().resolve(0.0);
                    let boxes = self
                        .id_map
                        .iter()
                        .map(|(id, (node_id, _))| (id.clone(), self.abs_box(*node_id)))
                        .collect();
                    let _ = reply.send(boxes);
                }
            }
        }
    }
}

/// A parsed, reusable document. Parsing and the first style pass happen once
/// in the constructor; `set_text`/`set_style`/`set_attribute` mutate elements
/// by their `id` attribute, and `render_png`/`render_rgba` re-resolve and
/// paint — typically ~1ms for widget-sized output.
///
/// The document lives on a dedicated worker thread (it is not thread-safe
/// itself), so a `Template` can be freely shared across Python threads;
/// operations are serialized in call order.
///
/// The id map is built once at construction; if several elements share an
/// `id` (invalid HTML), the last one in tree order wins.
#[pyclass]
struct Template {
    tx: Mutex<std::sync::mpsc::Sender<TemplateCmd>>,
}

impl Template {
    fn send(&self, cmd: TemplateCmd) -> PyResult<()> {
        self.tx
            .lock()
            .map_err(|_| PyValueError::new_err("template lock poisoned"))?
            .send(cmd)
            .map_err(|_| PyValueError::new_err("template worker has shut down"))
    }

    fn mutate(
        &self,
        py: Python<'_>,
        make: impl FnOnce(std::sync::mpsc::Sender<Result<(), String>>) -> TemplateCmd,
    ) -> PyResult<()> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send(make(reply_tx))?;
        py.detach(move || reply_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker has shut down"))?
            .map_err(PyValueError::new_err)
    }

    fn render(&self, py: Python<'_>, time: f64) -> PyResult<RenderResult> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send(TemplateCmd::Render { time, reply: reply_tx })?;
        py.detach(move || reply_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker has shut down"))
    }

    fn request_frames(
        &self,
        py: Python<'_>,
        times: Vec<f64>,
    ) -> PyResult<(u32, u32, Vec<Vec<u8>>)> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send(TemplateCmd::RenderFrames { times, reply: reply_tx })?;
        py.detach(move || reply_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker has shut down"))
    }
}

fn validate_times(times: &[f64]) -> PyResult<()> {
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
    Ok(())
}

#[pymethods]
impl Template {
    #[new]
    #[pyo3(signature = (html, *, width, height, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=false, css=None, css_vars=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
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
        css: Option<&str>,
        css_vars: Option<std::collections::HashMap<String, String>>,
    ) -> PyResult<Self> {
        validate(width, Some(height), scale)?;
        let color_scheme = parse_color_scheme(color_scheme)?;
        let background = parse_background(background)?;
        let html = compose_html(html, css, css_vars)?;
        let fonts = fonts.unwrap_or_default();

        let (tx, rx) = std::sync::mpsc::channel::<TemplateCmd>();
        let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<(), String>>();

        std::thread::Builder::new()
            .name("blitz-py-template".into())
            .stack_size(64 << 20)
            .spawn(move || {
                let built = build_document(
                    &html,
                    width,
                    height,
                    scale,
                    color_scheme,
                    base_url,
                    fonts,
                    default_font_family,
                    allow_file_urls,
                );
                match built {
                    Ok(mut document) => {
                        document.as_mut().resolve(0.0);
                        document.as_mut().resolve(0.0);
                        let id_map = build_id_map(document.as_ref());
                        let _ = init_tx.send(Ok(()));
                        TemplateWorker {
                            document,
                            scale,
                            physical_width: (width as f64 * scale as f64) as u32,
                            physical_height: (height as f64 * scale as f64) as u32,
                            background,
                            id_map,
                        }
                        .run(rx);
                    }
                    Err(e) => {
                        let _ = init_tx.send(Err(e.to_string()));
                    }
                }
            })
            .map_err(|e| PyValueError::new_err(format!("failed to spawn worker: {e}")))?;

        py.detach(move || init_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker died during construction"))?
            .map_err(PyValueError::new_err)?;
        Ok(Template { tx: Mutex::new(tx) })
    }

    /// Replace the text content of the element with the given `id`.
    /// The element must contain exactly one text node.
    fn set_text(&self, py: Python<'_>, id: &str, text: &str) -> PyResult<()> {
        let (id, text) = (id.to_string(), text.to_string());
        self.mutate(py, |reply| TemplateCmd::SetText { id, text, reply })
    }

    /// Set an inline style property (e.g. `set_style("bar", "width", "62%")`).
    fn set_style(&self, py: Python<'_>, id: &str, name: &str, value: &str) -> PyResult<()> {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(PyValueError::new_err(format!(
                "invalid css property name '{name}'"
            )));
        }
        let (id, name, value) = (id.to_string(), name.to_string(), value.to_string());
        self.mutate(py, |reply| TemplateCmd::SetStyle { id, name, value, reply })
    }

    /// Set an attribute on the element with the given `id`.
    fn set_attribute(&self, py: Python<'_>, id: &str, name: &str, value: &str) -> PyResult<()> {
        if name.eq_ignore_ascii_case("id") {
            // The id -> node map is built once at construction; renaming ids
            // would silently desynchronize it.
            return Err(PyValueError::new_err(
                "changing the 'id' attribute of a template element is not supported",
            ));
        }
        let (id, name, value) = (id.to_string(), name.to_string(), value.to_string());
        self.mutate(py, |reply| TemplateCmd::SetAttr { id, name, value, reply })
    }

    /// Replace the children of the element with the given `id` by parsing an
    /// HTML fragment. Ids inside the replaced region are re-indexed.
    fn set_html(&self, py: Python<'_>, id: &str, html: &str) -> PyResult<()> {
        let (id, html) = (id.to_string(), html.to_string());
        self.mutate(py, |reply| TemplateCmd::SetHtml { id, html, reply })
    }

    /// Batch text update: `tpl.update(temp="21.5°", hum="48%")` sets the text
    /// of the elements with ids `temp` and `hum` in one round-trip.
    /// Validates every id before applying — a failing batch changes nothing.
    #[pyo3(signature = (**kwargs))]
    fn update(&self, py: Python<'_>, kwargs: Option<Bound<'_, pyo3::types::PyDict>>) -> PyResult<()> {
        let Some(kwargs) = kwargs else { return Ok(()) };
        let mut updates = Vec::with_capacity(kwargs.len());
        for (k, v) in kwargs.iter() {
            updates.push((k.extract::<String>()?, v.extract::<String>()?));
        }
        if updates.is_empty() {
            return Ok(());
        }
        self.mutate(py, |reply| TemplateCmd::UpdateTexts { updates, reply })
    }

    /// Render the current document state at multiple animation timestamps.
    /// Returns `(width, height, [rgba_bytes, ...])`.
    #[pyo3(signature = (*, times))]
    fn render_frames<'py>(
        &self,
        py: Python<'py>,
        times: Vec<f64>,
    ) -> PyResult<(u32, u32, Vec<Bound<'py, PyBytes>>)> {
        validate_times(&times)?;
        let (w, h, frames) = self.request_frames(py, times)?;
        let frames = frames.iter().map(|f| PyBytes::new(py, f)).collect();
        Ok((w, h, frames))
    }

    /// Render the current document state as an infinitely-looping GIF.
    #[pyo3(signature = (*, times, colors=64))]
    fn render_gif<'py>(
        &self,
        py: Python<'py>,
        times: Vec<f64>,
        colors: u16,
    ) -> PyResult<Bound<'py, PyBytes>> {
        validate_gif_args(&times, colors)?;
        let (w, h, frames) = self.request_frames(py, times.clone())?;
        let gif = py.detach(|| encode_gif(w, h, &frames, &times, colors))?;
        Ok(PyBytes::new(py, &gif))
    }

    /// Render the current state to JPEG bytes. `time` is the animation clock.
    #[pyo3(signature = (*, quality=90, time=0.0))]
    fn render_jpeg<'py>(
        &self,
        py: Python<'py>,
        quality: u8,
        time: f64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        validate_quality(quality)?;
        let result = self.render(py, time)?;
        let jpeg = py.detach(|| encode_jpeg(&result, quality))?;
        Ok(PyBytes::new(py, &jpeg))
    }

    /// Final laid-out rect of the element with this `id`, as
    /// `(x, y, width, height)` in CSS pixels.
    fn get_box(&self, py: Python<'_>, id: &str) -> PyResult<(f64, f64, f64, f64)> {
        let id = id.to_string();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send(TemplateCmd::GetBox { id, reply: reply_tx })?;
        py.detach(move || reply_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker has shut down"))?
            .map_err(PyValueError::new_err)
    }

    /// Laid-out rects of every element with an `id`, as
    /// `{id: (x, y, width, height)}` in CSS pixels.
    fn boxes(
        &self,
        py: Python<'_>,
    ) -> PyResult<std::collections::HashMap<String, (f64, f64, f64, f64)>> {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.send(TemplateCmd::Boxes { reply: reply_tx })?;
        let boxes = py
            .detach(move || reply_rx.recv())
            .map_err(|_| PyValueError::new_err("template worker has shut down"))?;
        Ok(boxes.into_iter().collect())
    }

    /// Render the current state to PNG bytes. `time` is the animation clock.
    #[pyo3(signature = (*, time=0.0))]
    fn render_png<'py>(&self, py: Python<'py>, time: f64) -> PyResult<Bound<'py, PyBytes>> {
        let result = self.render(py, time)?;
        let png = py.detach(|| encode_png(&result));
        Ok(PyBytes::new(py, &png))
    }

    /// Render the current state to raw RGBA. `time` is the animation clock.
    #[pyo3(signature = (*, time=0.0))]
    fn render_rgba<'py>(
        &self,
        py: Python<'py>,
        time: f64,
    ) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
        let result = self.render(py, time)?;
        Ok((result.width, result.height, PyBytes::new(py, &result.rgba)))
    }
}

// ---------------------------------------------------------------------------
// Layered compositing: render several documents/templates into one surface.

enum LayerJob {
    Html {
        html: String,
        width: u32,
        height: u32,
        scale: f32,
        color_scheme: ColorScheme,
        background: Option<blitz_dom::util::Color>,
        base_url: Option<String>,
        fonts: Vec<Vec<u8>>,
        default_font_family: Option<String>,
        allow_file_urls: bool,
        time: f64,
    },
    Tpl {
        sender: std::sync::mpsc::Sender<TemplateCmd>,
        time: f64,
    },
}

struct LayerSpec {
    job: LayerJob,
    x: i64,
    y: i64,
    opacity: f32,
    blur: f32,
    tint: Option<(u8, u8, u8)>,
}

fn dict_get<'py, T: for<'a> pyo3::FromPyObject<'a, 'py>>(
    dict: &Bound<'py, pyo3::types::PyDict>,
    key: &str,
) -> PyResult<Option<T>> {
    match dict.get_item(key)? {
        Some(v) if !v.is_none() => Ok(Some(v.extract::<T>().map_err(|_| {
            PyValueError::new_err(format!("layer key '{key}' has an invalid type"))
        })?)),
        _ => Ok(None),
    }
}

fn parse_layers(layers: &[Bound<'_, pyo3::types::PyDict>]) -> PyResult<Vec<LayerSpec>> {
    let mut specs = Vec::with_capacity(layers.len());
    for dict in layers {
        let x: i64 = dict_get(dict, "x")?.unwrap_or(0);
        let y: i64 = dict_get(dict, "y")?.unwrap_or(0);
        let opacity: f32 = dict_get(dict, "opacity")?.unwrap_or(1.0);
        if !(0.0..=1.0).contains(&opacity) {
            return Err(PyValueError::new_err("layer opacity must be in 0..=1"));
        }
        let blur: f32 = dict_get(dict, "blur")?.unwrap_or(0.0);
        if !(0.0..=100.0).contains(&blur) {
            return Err(PyValueError::new_err("layer blur must be in 0..=100 px"));
        }
        let tint = match dict_get::<String>(dict, "tint")? {
            Some(hex) => {
                let (r, g, b, _) = parse_hex_rgba(&hex)?;
                Some((r, g, b))
            }
            None => None,
        };
        let time: f64 = dict_get(dict, "time")?.unwrap_or(0.0);

        let template: Option<Bound<'_, Template>> = match dict.get_item("template")? {
            Some(v) if !v.is_none() => Some(v.downcast_into::<Template>().map_err(|_| {
                PyValueError::new_err("layer 'template' must be a blitz_py.Template")
            })?),
            _ => None,
        };
        let html: Option<String> = dict_get(dict, "html")?;
        let job = match (template, html) {
            (Some(tpl), None) => {
                let sender = tpl
                    .borrow()
                    .tx
                    .lock()
                    .map_err(|_| PyValueError::new_err("template lock poisoned"))?
                    .clone();
                LayerJob::Tpl { sender, time }
            }
            (None, Some(html)) => {
                let width: u32 = dict_get(dict, "width")?
                    .ok_or_else(|| PyValueError::new_err("html layer requires 'width'"))?;
                let height: u32 = dict_get(dict, "height")?
                    .ok_or_else(|| PyValueError::new_err("html layer requires 'height'"))?;
                let scale: f32 = dict_get(dict, "scale")?.unwrap_or(1.0);
                validate(width, Some(height), scale)?;
                let color_scheme =
                    parse_color_scheme(&dict_get::<String>(dict, "color_scheme")?
                        .unwrap_or_else(|| "light".into()))?;
                // Layers default to a transparent background so they stack.
                let background =
                    parse_background(dict_get::<String>(dict, "background")?.as_deref())?;
                let css: Option<String> = dict_get(dict, "css")?;
                let css_vars: Option<std::collections::HashMap<String, String>> =
                    dict_get(dict, "css_vars")?;
                let html = compose_html(&html, css.as_deref(), css_vars)?;
                LayerJob::Html {
                    html,
                    width,
                    height,
                    scale,
                    color_scheme,
                    background,
                    base_url: dict_get(dict, "base_url")?,
                    fonts: dict_get(dict, "fonts")?.unwrap_or_default(),
                    default_font_family: dict_get(dict, "default_font_family")?,
                    allow_file_urls: dict_get(dict, "allow_file_urls")?.unwrap_or(false),
                    time,
                }
            }
            (Some(_), Some(_)) => {
                return Err(PyValueError::new_err(
                    "layer must have either 'template' or 'html', not both",
                ));
            }
            (None, None) => {
                return Err(PyValueError::new_err(
                    "layer requires a 'template' or 'html' key",
                ));
            }
        };
        specs.push(LayerSpec { job, x, y, opacity, blur, tint });
    }
    Ok(specs)
}

/// Three-pass box blur (≈ gaussian), separable, on premultiplied RGBA.
fn box_blur(buf: &mut [u8], w: usize, h: usize, radius: f32) {
    let r = radius.round() as usize;
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        // Horizontal pass buf -> tmp
        for y in 0..h {
            let row = y * w;
            let mut sums = [0u32; 4];
            let window = |x: isize| -> usize { (row + x.clamp(0, w as isize - 1) as usize) * 4 };
            for x in -(r as isize)..=(r as isize) {
                let p = window(x);
                for c in 0..4 {
                    sums[c] += buf[p + c] as u32;
                }
            }
            let count = (2 * r + 1) as u32;
            for x in 0..w {
                let out = (row + x) * 4;
                for c in 0..4 {
                    tmp[out + c] = (sums[c] / count) as u8;
                }
                let leaving = window(x as isize - r as isize);
                let entering = window(x as isize + r as isize + 1);
                for c in 0..4 {
                    sums[c] = sums[c] + buf[entering + c] as u32 - buf[leaving + c] as u32;
                }
            }
        }
        // Vertical pass tmp -> buf
        for x in 0..w {
            let mut sums = [0u32; 4];
            let window = |y: isize| -> usize {
                (y.clamp(0, h as isize - 1) as usize * w + x) * 4
            };
            for y in -(r as isize)..=(r as isize) {
                let p = window(y);
                for c in 0..4 {
                    sums[c] += tmp[p + c] as u32;
                }
            }
            let count = (2 * r + 1) as u32;
            for y in 0..h {
                let out = (y * w + x) * 4;
                for c in 0..4 {
                    buf[out + c] = (sums[c] / count) as u8;
                }
                let leaving = window(y as isize - r as isize);
                let entering = window(y as isize + r as isize + 1);
                for c in 0..4 {
                    sums[c] = sums[c] + tmp[entering + c] as u32 - tmp[leaving + c] as u32;
                }
            }
        }
    }
}

fn run_layers(
    specs: Vec<LayerSpec>,
    width: u32,
    height: u32,
    background: (u8, u8, u8, u8),
) -> PyResult<RenderResult> {
    let (cw, ch) = (width as usize, height as usize);
    // Canvas is premultiplied RGBA, matching the renderer's output.
    let (br, bg_, bb, ba) = background;
    let a = ba as u32;
    let mut canvas = vec![0u8; cw * ch * 4];
    for px in canvas.chunks_exact_mut(4) {
        px[0] = ((br as u32 * a) / 255) as u8;
        px[1] = ((bg_ as u32 * a) / 255) as u8;
        px[2] = ((bb as u32 * a) / 255) as u8;
        px[3] = ba;
    }

    for spec in specs {
        let result = match spec.job {
            LayerJob::Html {
                html,
                width,
                height,
                scale,
                color_scheme,
                background,
                base_url,
                fonts,
                default_font_family,
                allow_file_urls,
                time,
            } => {
                let (w, h, mut frames) = render_frames_impl(
                    &html,
                    width,
                    height,
                    &[time],
                    scale,
                    color_scheme,
                    background,
                    base_url,
                    fonts,
                    default_font_family,
                    allow_file_urls,
                )?;
                RenderResult { rgba: frames.remove(0), width: w, height: h }
            }
            LayerJob::Tpl { sender, time } => {
                let (reply_tx, reply_rx) = std::sync::mpsc::channel();
                sender
                    .send(TemplateCmd::Render { time, reply: reply_tx })
                    .map_err(|_| PyValueError::new_err("template worker has shut down"))?;
                reply_rx
                    .recv()
                    .map_err(|_| PyValueError::new_err("template worker has shut down"))?
            }
        };

        let mut rgba = result.rgba;
        let (lw, lh) = (result.width as usize, result.height as usize);
        if spec.blur > 0.0 {
            box_blur(&mut rgba, lw, lh, spec.blur);
        }
        if let Some((tr, tg, tb)) = spec.tint {
            // Recolor keeping coverage: rgb := tint * alpha (premultiplied).
            for px in rgba.chunks_exact_mut(4) {
                let a = px[3] as u32;
                px[0] = ((tr as u32 * a) / 255) as u8;
                px[1] = ((tg as u32 * a) / 255) as u8;
                px[2] = ((tb as u32 * a) / 255) as u8;
            }
        }
        if spec.opacity < 1.0 {
            let f = (spec.opacity * 255.0) as u32;
            for px in rgba.chunks_exact_mut(4) {
                for c in 0..4 {
                    px[c] = ((px[c] as u32 * f) / 255) as u8;
                }
            }
        }

        // Premultiplied source-over, clipped to the canvas.
        for ly in 0..lh {
            let cy = spec.y + ly as i64;
            if cy < 0 || cy >= ch as i64 {
                continue;
            }
            for lx in 0..lw {
                let cx = spec.x + lx as i64;
                if cx < 0 || cx >= cw as i64 {
                    continue;
                }
                let s = (ly * lw + lx) * 4;
                let d = (cy as usize * cw + cx as usize) * 4;
                let sa = rgba[s + 3] as u32;
                if sa == 0 {
                    continue;
                }
                let inv = 255 - sa;
                for c in 0..4 {
                    canvas[d + c] =
                        (rgba[s + c] as u32 + (canvas[d + c] as u32 * inv) / 255) as u8;
                }
            }
        }
    }
    Ok(RenderResult { rgba: canvas, width, height })
}

#[allow(clippy::too_many_arguments)]
fn layers_common(
    py: Python<'_>,
    layers: Vec<Bound<'_, pyo3::types::PyDict>>,
    width: u32,
    height: u32,
    background: &str,
) -> PyResult<RenderResult> {
    validate(width, Some(height), 1.0)?;
    let background = parse_hex_rgba(background)?;
    let specs = parse_layers(&layers)?;
    py.detach(|| with_render_stack(|| run_layers(specs, width, height, background)))
}

/// Composite several documents and/or templates into one surface.
///
/// Each layer is a dict with either `template` (a `Template`) or `html`
/// (+ required `width`/`height` and the usual render options), plus optional
/// `x`, `y`, `opacity`, `blur` (px — e.g. glow underlays), `tint` (recolor
/// keeping alpha) and `time` (animation clock). Layers paint in list order
/// and are clipped to their rect and the canvas. Returns `(w, h, rgba)`.
#[pyfunction]
#[pyo3(signature = (layers, *, width, height, background="#000000"))]
fn render_layers<'py>(
    py: Python<'py>,
    layers: Vec<Bound<'py, pyo3::types::PyDict>>,
    width: u32,
    height: u32,
    background: &str,
) -> PyResult<(u32, u32, Bound<'py, PyBytes>)> {
    let result = layers_common(py, layers, width, height, background)?;
    Ok((result.width, result.height, PyBytes::new(py, &result.rgba)))
}

/// `render_layers`, encoded as PNG bytes.
#[pyfunction]
#[pyo3(signature = (layers, *, width, height, background="#000000"))]
fn render_layers_png<'py>(
    py: Python<'py>,
    layers: Vec<Bound<'py, pyo3::types::PyDict>>,
    width: u32,
    height: u32,
    background: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let result = layers_common(py, layers, width, height, background)?;
    let png = py.detach(|| encode_png(&result));
    Ok(PyBytes::new(py, &png))
}

/// `render_layers`, encoded as JPEG bytes.
#[pyfunction]
#[pyo3(signature = (layers, *, width, height, background="#000000", quality=90))]
fn render_layers_jpeg<'py>(
    py: Python<'py>,
    layers: Vec<Bound<'py, pyo3::types::PyDict>>,
    width: u32,
    height: u32,
    background: &str,
    quality: u8,
) -> PyResult<Bound<'py, PyBytes>> {
    validate_quality(quality)?;
    let result = layers_common(py, layers, width, height, background)?;
    let jpeg = py.detach(|| encode_jpeg(&result, quality))?;
    Ok(PyBytes::new(py, &jpeg))
}

#[pymodule]
fn blitz_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(render_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(render_frames, m)?)?;
    m.add_function(wrap_pyfunction!(render_jpeg, m)?)?;
    m.add_function(wrap_pyfunction!(render_gif, m)?)?;
    m.add_function(wrap_pyfunction!(measure_text, m)?)?;
    m.add_function(wrap_pyfunction!(measure_text_lines, m)?)?;
    m.add_function(wrap_pyfunction!(register_fonts, m)?)?;
    m.add_function(wrap_pyfunction!(ellipsize, m)?)?;
    m.add_function(wrap_pyfunction!(line_clamp, m)?)?;
    m.add_function(wrap_pyfunction!(fit_font_size, m)?)?;
    m.add_function(wrap_pyfunction!(wrap_balanced, m)?)?;
    m.add_function(wrap_pyfunction!(render_layers, m)?)?;
    m.add_function(wrap_pyfunction!(render_layers_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_layers_jpeg, m)?)?;
    m.add_class::<Template>()?;
    Ok(())
}
