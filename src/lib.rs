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
            if value.contains(['{', '}']) {
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
    })?;
    let frames = frames.iter().map(|f| PyBytes::new(py, f)).collect();
    Ok((w, h, frames))
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
    Render {
        time: f64,
        reply: std::sync::mpsc::Sender<RenderResult>,
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

#[pymodule]
fn blitz_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(render_png, m)?)?;
    m.add_function(wrap_pyfunction!(render_rgba, m)?)?;
    m.add_function(wrap_pyfunction!(render_frames, m)?)?;
    m.add_class::<Template>()?;
    Ok(())
}
