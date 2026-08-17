# Changelog

## Unreleased

### Changed
- **Engine bump: Blitz `main` @ `01a6c4b` (2026-08-17) via git pin** — ~140 commits past `0.3.0-beta.1`, no crates.io release exists yet. Brings Stylo 0.20, Taffy 0.13, parley/fontique 0.11.1, anyrender 0.13 / vello_cpu 0.16, and a long list of layout and robustness fixes: no more panics on documents without a root element, on unresolvable `@font-face` URLs, on degenerate aspect ratios; correct resource gating with a synchronous `NetProvider` (#636/#695); table sizing (border-spacing, fixed columns, cell padding with floats, `calc()` widths); floats no longer split the inline flow; replaced-element intrinsic sizing (img/svg/canvas/iframe); block-axis `align-content`, `order` on `::before/::after`, `safe` alignment; `background-attachment: fixed`, SVG background-image sizing per CSS, `contain: paint`, `clip-path` corner radii; `<iframe>`, comment nodes, scoped `querySelector`. Incremental layout is now on by default. Renders of the gallery samples are pixel-identical to 0.4.2; warm-render times are unchanged to slightly faster.
- The fontconfig zero-fonts and `FontContext::new()` leak workarounds stay: fontique 0.11.1 still carries both issues (see `docs/UPSTREAM.md`).

## 0.4.2 — 2026-08-07

### Fixed
- **No more crash on hosts with fontconfig but zero installed fonts** (e.g. fresh Home Assistant base images). fontique 0.10's fontconfig backend panics (`unwrap` on `NoMatch`) while enumerating system fonts on such hosts, and the panic escaped as a `pyo3_runtime.PanicException` (a `BaseException`) from the first blitz-py call — and again from every retry, since it aborted the one-time font-collection init. The system-font scan is now probed under `catch_unwind`; on failure blitz-py logs one warning line and continues with the bundled Inter only (identical rendering for documents that don't reference system fonts — losing only system-font lookups and CJK fallback). Upstream has fixed the `unwrap` on parley main, but no fontique release (≤ 0.11) contains it. Covered by a new CI job that installs the wheel in Alpine with fontconfig present and no fonts. Reported by an integration hitting it on a fresh HA container — thanks!

## 0.4.1 — 2026-08-07

### Fixed
- **Animation timestamps are now absolute.** The engine anchors its animation clock at the first style resolve, so `render_frames`/`render_gif` silently treated `times` as relative to `times[0]` (invisible when starting at 0.0, which every example did), and `render_layers`' per-layer `time` was ignored for `html` layers entirely. The binding now anchors at t=0 and resolves each timestamp explicitly. Reported by an integration porting onto `render_layers` — thanks!

## 0.4.0 — 2026-08-07

### Added
- **`render_layers` / `render_layers_png` / `render_layers_jpeg`**: composite documents and `Template`s into one surface — positions, explicit paint order, premultiplied alpha, and per-rect clipping in Rust. Per-layer `opacity`, `blur`, and `tint` enable glow/shadow effects (including text glow) that CSS can't express in this engine yet.
- **`Template.get_box(id)` / `Template.boxes()`**: post-layout rects in CSS px — replaces Python-side mirrors of CSS sizing math.
- **`register_fonts(fonts, default_family=None)`**: process-wide font registration; returns family names.
- **Text utilities on the engine's shaper**: `ellipsize`, `line_clamp` (multi-line), `fit_font_size` (with `wrap`/`max_height`), `wrap_balanced` (`text-wrap: balance`), `measure_text_lines` (per-line metrics).

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
