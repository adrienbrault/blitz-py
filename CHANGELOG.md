# Changelog

## 0.3.0 — 2026-08-07

### Added
- **Native `render_jpeg`** (module + `Template` method): ~0.5ms encode, `quality=` param — no Pillow round-trip.
- **Native `render_gif`** (module + `Template` method): shared NeuQuant palette, no dithering, transparency-based inter-frame deltas — the 38-frame widget loop is ~16KB (smaller than the Pillow route) in one call.
- **`Template.set_html(id, fragment)`**: replace an element's children with a parsed HTML fragment; ids inside the region are re-indexed.
- **`Template.update(**id_to_text)`**: batch text updates in one worker round-trip; validated up-front, atomic on failure.
- **`Template.render_frames(times=...)`**: animation frames from the current (mutated) document state — mutate between calls for data-driven animations.
- **`measure_text(text, *, font_size, ...)`**: text measurement via the engine's own shaper (Parley + the rendering font collection) — one source of truth for Python-side ellipsis/fitting logic. Supports CSS font-family lists, weight, letter-spacing, and `max_width` wrapping.
- README recipes: OG/social cards (with sample), email-HTML previews, exact-hash visual snapshot tests.
- CI: musl wheels are now *tested* in an Alpine container (previously only built); `windows-aarch64` wheels added.
- Explicit 32-bit (armv7l) policy: no wheels — `anyrender` 0.11 does not compile on 32-bit targets (documented in README and docs/UPSTREAM.md).

## 0.2.0 — 2026-08-07

### Added
- **`Template` class**: parse once, mutate by element `id` (`set_text` / `set_style` / `set_attribute`), re-render in ~0.4ms. Thread-safe (document lives on a dedicated worker thread).
- **Auto height**: `render_png` / `render_rgba` accept `height=None` and size the canvas to the laid-out content.
- **`css=`** parameter: extra CSS appended after document styles.
- **`css_vars=`** parameter: CSS custom properties injected on `:root`.
- **Type stubs** (`py.typed` + `.pyi`) for autocomplete and type checking.
- Golden-image determinism check in CI: Linux/macOS/Windows renders asserted byte-identical.
- Hostile-input test corpus and a cargo-fuzz target for the render pipeline.
- Thread-scaling and GIF benchmarks in `examples/bench.py`.

### Changed
- Release profile switched to `opt-level="s"` + fat LTO: ~19% smaller binaries, slightly faster renders.

### Notes
- `Template.set_style` rewrites the element's `style` attribute (blitz 0.3.0-beta.1's style-property mutation misses layout invalidation; see `docs/UPSTREAM.md`).

## 0.1.0 — 2026-08-07

First release.

- `render_png` / `render_rgba` / `render_frames` over Blitz 0.3.0-beta.1 (Stylo + Taffy + Parley + vello_cpu).
- Bundled Inter (OFL 1.1) as cross-platform default font; custom fonts via `fonts=`; `@font-face` with `data:`/`file://` sources.
- Deterministic offline rendering: `data:` URIs supported, network intentionally unsupported, `file://` opt-in.
- CSS animations on an explicit clock via `render_frames` → GIFs with Pillow.
- abi3 wheels (Python ≥3.10): manylinux/musllinux x86_64+aarch64, macOS arm64+x86_64, Windows x64.
