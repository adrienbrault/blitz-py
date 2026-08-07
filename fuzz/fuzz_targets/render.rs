//! Fuzz the parse -> style -> layout -> paint pipeline with arbitrary HTML.
//! Run (nightly): cargo +nightly fuzz run render -- -max_len=65536
#![no_main]

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(html) = std::str::from_utf8(data) else { return };
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(64, 64, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.as_mut().resolve(0.0);
    doc.as_mut().resolve(0.5);
    let _ = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| paint_scene(scene, doc.as_mut(), 1.0, 64, 64, 0, 0),
        64,
        64,
    );
});
