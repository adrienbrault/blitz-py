//! Memory-leak isolation harness. Run stages with:
//!   cargo run --release --example leak -- <stage>
//! Stages: fontctx | fontctx_reg | doc | paint | full, plus per-thread
//! variants that run each iteration on a fresh thread (fonts cloned from one
//! shared collection, so the FontContext::new() leak doesn't confound):
//! thread_noop | thread_parse | thread_doc | thread_full | doc_shared |
//! full_shared

use std::process::Command;
use std::sync::Arc;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};

const HTML: &str =
    "<body style='background:#123'><div style='width:100px;height:100px;background:#f00'></div>";
const W: u32 = 480;
const H: u32 = 480;

fn rss_mb() -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .unwrap()
        / 1024.0
}

fn font_ctx(register: bool) -> parley::FontContext {
    let mut ctx = parley::FontContext::new();
    if register {
        static FONT: &[u8] = include_bytes!("../assets/InterVariable.ttf");
        let blob = parley::fontique::Blob::new(Arc::new(FONT));
        ctx.collection.register_fonts(blob, None);
    }
    ctx
}

fn make_doc(register: bool) -> HtmlDocument {
    HtmlDocument::from_html(
        HTML,
        DocumentConfig {
            viewport: Some(Viewport::new(W, H, 1.0, ColorScheme::Light)),
            font_ctx: Some(font_ctx(register)),
            ..Default::default()
        },
    )
}

fn paint(doc: &mut HtmlDocument) -> Vec<u8> {
    render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, doc.as_mut(), 1.0, W, H, 0, 0),
        W,
        H,
    )
}

/// A document whose font context clones a shared collection instead of
/// scanning platform fonts (which leaks on its own; see `fontctx`).
fn make_doc_shared(base: &parley::fontique::Collection) -> HtmlDocument {
    HtmlDocument::from_html(
        HTML,
        DocumentConfig {
            viewport: Some(Viewport::new(W, H, 1.0, ColorScheme::Light)),
            font_ctx: Some(parley::FontContext {
                collection: base.clone(),
                source_cache: Default::default(),
            }),
            ..Default::default()
        },
    )
}

fn doc_shared(base: &parley::fontique::Collection, paint_too: bool) {
    let mut doc = make_doc_shared(base);
    doc.as_mut().resolve(0.0);
    doc.as_mut().resolve(0.0);
    if paint_too {
        let _ = paint(&mut doc);
    }
}

fn main() {
    let stage = std::env::args().nth(1).unwrap_or_else(|| "full".into());
    let mut reused_doc = make_doc(true);
    let base = font_ctx(true).collection;
    println!("stage={stage} start rss={:.0}MB", rss_mb());
    for i in 1..=600u32 {
        match stage.as_str() {
            "fontctx" => {
                let _ = font_ctx(false);
            }
            "fontctx_reg" => {
                let _ = font_ctx(true);
            }
            "doc" => {
                let mut doc = make_doc(true);
                doc.as_mut().resolve(0.0);
                doc.as_mut().resolve(0.0);
            }
            "paint" => {
                let _ = paint(&mut reused_doc);
            }
            "full" => {
                let mut doc = make_doc(true);
                doc.as_mut().resolve(0.0);
                doc.as_mut().resolve(0.0);
                let _ = paint(&mut doc);
            }
            "doc_shared" => doc_shared(&base, false),
            "full_shared" => doc_shared(&base, true),
            "thread_noop" => std::thread::spawn(|| {}).join().unwrap(),
            "thread_parse" => {
                let base = base.clone();
                std::thread::spawn(move || drop(make_doc_shared(&base)))
                    .join()
                    .unwrap();
            }
            "thread_doc" | "thread_full" => {
                let base = base.clone();
                let paint_too = stage == "thread_full";
                std::thread::spawn(move || doc_shared(&base, paint_too))
                    .join()
                    .unwrap();
            }
            other => panic!("unknown stage {other}"),
        }
        if i % 150 == 0 {
            println!("  iter {i}: rss={:.0}MB", rss_mb());
        }
    }
}
