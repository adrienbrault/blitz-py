//! Memory-leak isolation harness. Run stages with:
//!   cargo run --release --example leak -- <stage>
//! Stages: fontctx | fontctx_reg | doc | paint | full

use std::process::Command;
use std::sync::Arc;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};

const HTML: &str = "<body style='background:#123'><div style='width:100px;height:100px;background:#f00'></div>";
const W: u32 = 480;
const H: u32 = 480;

fn rss_mb() -> f64 {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap() / 1024.0
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

fn main() {
    let stage = std::env::args().nth(1).unwrap_or_else(|| "full".into());
    let mut reused_doc = make_doc(true);
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
            other => panic!("unknown stage {other}"),
        }
        if i % 150 == 0 {
            println!("  iter {i}: rss={:.0}MB", rss_mb());
        }
    }
}

