//! Probe: does `resolve(t)` advance CSS animations deterministically?
//! Run: cargo run --release --example anim

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};

const HTML: &str = r#"
<style>
  body { margin: 0; background: #fff; }
  .box { width: 40px; height: 40px; background: #e11; border-radius: 8px;
         animation: slide 2s linear infinite; }
  @keyframes slide {
    from { transform: translateX(0); }
    to   { transform: translateX(160px); }
  }
  .fade { width: 40px; height: 40px; background: #11e;
          animation: fade 2s linear infinite; }
  @keyframes fade { from { opacity: 1; } to { opacity: 0; } }
</style>
<body><div class="box"></div><div class="fade"></div></body>
"#;

const W: u32 = 220;
const H: u32 = 100;

fn render_at(doc: &mut HtmlDocument, t: f64) -> Vec<u8> {
    doc.as_mut().resolve(t);
    render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, doc.as_mut(), 1.0, W, H, 0, 0),
        W,
        H,
    )
}

fn first_red_x(buf: &[u8]) -> Option<u32> {
    // scan row y=20 for the red box
    for x in 0..W {
        let i = ((20 * W + x) * 4) as usize;
        if buf[i] > 180 && buf[i + 1] < 100 {
            return Some(x);
        }
    }
    None
}

fn main() {
    let mut doc = HtmlDocument::from_html(
        HTML,
        DocumentConfig {
            viewport: Some(Viewport::new(W, H, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    for t in [0.0, 0.5, 1.0, 1.5, 2.0] {
        let buf = render_at(&mut doc, t);
        let x = first_red_x(&buf);
        // blue box alpha at y=60
        let i = ((60 * W + 10) * 4) as usize;
        println!(
            "t={t:>4}s  red box x={:?}  blue pixel rgb=({},{},{})",
            x,
            buf[i],
            buf[i + 1],
            buf[i + 2]
        );
    }
}
