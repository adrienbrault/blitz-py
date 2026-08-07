# Changelog

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
