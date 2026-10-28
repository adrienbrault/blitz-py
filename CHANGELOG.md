# Changelog

## 0.6.0 — 2026-10-28

### Changed
- **Engine: Blitz `0.3.0-beta.4` from crates.io** (replaces the git pin on `main` @ `01a6c4b`; 289 commits further). Brings Taffy 0.15, Parley/fontique 0.12, anyrender 0.14 / vello_cpu 0.18. Visible effects: inline SVG with `preserveAspectRatio="none"` now fills its CSS box (chart sparklines span their container), `object-fit` applies to SVG images, CSS animations apply to `<svg>` elements directly, and format-less `data:` TTF `@font-face` sources load.
- **Line heights are pixel-snapped** (Parley 0.12): `normal` line-height for 16px Inter goes from 19.36 to 20px, so text shifts by ≤1px and `measure_text` heights change accordingly. Golden hashes differ from 0.5.0; renders stay byte-identical across platforms.
- PyO3 0.30 (Python 3.15 support), png 0.18, refreshed lockfile. CI now tests the abi3 wheel on Python 3.10–3.15 and on Alpine with 3.13/3.14.

### Added
- **`resources=`**: a dict of URL → `bytes` served in-memory to `<img>`, stylesheets, CSS `url()` and `@font-face` — no base64 in the HTML, no filesystem access. Keys resolve against `base_url` like `src` does (`{"logo.png": data}` answers `src="logo.png"`). Accepted by every render function, `Template` and `html` layers.
- **WOFF/WOFF2 web fonts** actually load now. The README promised them, but the engine's decoder was compiled out, so they silently fell back to the default face.
- **armv7 (32-bit ARM) wheels**, glibc and musl — the anyrender build failure that blocked them is fixed upstream.

### Fixed
- **Native memory leak of ~16KB per render call** ([geekmagic-hacs#209](https://github.com/adrienbrault/geekmagic-hacs/issues/209)): every one-shot `render_*`/`render_layers*` call ran on a freshly spawned 64MB-stack thread, and each new thread that renders leaves per-thread engine state behind that is never freed — ~16KB per call with 0.5.0 (~37KB with the 0.6.0 engine before this fix), i.e. ~5–6MB/hour for a display refreshing every 10s, GBs over days on a Raspberry Pi. Renders now run on a pool of reused large-stack threads that grows to the peak number of concurrent callers (so threads rendering in parallel still do), and growth is ~0KB/call. Panics still propagate to the caller, and a panicking render does not take its worker down. `Template` already reused its worker thread and was unaffected — but creating a new `Template` per render still spawns a thread, so keep templates long-lived.
- **No more `PanicException` on relative URLs without `base_url`** (`<img src="logo.png">`, `<link href="a.css">`, `<iframe src>`, `//host/x`). The engine's default base could not anchor relative URLs and panicked; the panic surfaced as `pyo3_runtime.PanicException`, a `BaseException` that `except Exception` misses. Present since 0.1.0. Documents now default to a hierarchical base; a `base_url` that cannot anchor relative URLs raises `ValueError`.
- **Image decode bombs**: the engine decodes images in full with no pixel limit, so a few-KB PNG declaring 9000×9000 allocated ~400MB. Images whose header declares more than 64M pixels are now skipped.
- **Animation memory is bounded per call**: `render_frames`/`render_gif` (module and `Template`) reject calls whose frames total more than 256M physical pixels (~1GB of RGBA); before, only per-frame size and frame count were checked. Peak memory under the budget is also lower (frames are freed as they are encoded or handed to Python).
- `file://` reads (opt-in) are capped at 64MB, so e.g. `file:///dev/zero` no longer reads forever.

### Removed
- The fontconfig zero-fonts `catch_unwind` probe: fontique 0.12 fixes the panic upstream. The fontless Alpine CI job stays as the regression test.

## 0.5.0 — 2026-08-17

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
