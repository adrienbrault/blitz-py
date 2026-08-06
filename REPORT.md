# blitz-py: spike results & recommended approach

**TL;DR: it works, and it's simpler than expected.** A ~300-line PyO3 binding over the published Blitz crates renders real HTML/CSS (flexbox, grid, gradients, border-radius, data-URI images, custom fonts) to PNG/RGBA in **~22ms per 240×240 frame**, CPU-only, no network, no browser. The full Rust stack compiles in ~75s. Recommended path: build this repo into a proper package published to PyPI with prebuilt abi3 wheels via `maturin-action` CI.

## What the spike proved (all verified locally, 2026-08-06)

| Question | Result |
|---|---|
| Do published crates suffice? | Yes — `blitz-{dom,html,paint,traits} 0.3.0-beta.1` + `anyrender_vello_cpu 0.14` from crates.io. No git dependencies, no C/C++ deps, pure Rust. |
| Render quality | Excellent for widget UI: flexbox, `linear-gradient`, `border-radius`, pills, emoji-free text all correct on first try. `scale=2.0` gives cheap supersampling. |
| Performance | 136ms cold (first render includes font discovery), **22ms warm** for 240×240. PNG ~15KB. |
| Images without network | `data:` URIs work via a tiny custom `NetProvider` (~25 lines) that decodes them synchronously. `file://` behind an opt-in flag. http(s) intentionally unsupported. |
| Custom fonts | `fonts=[ttf_bytes]` + `default_font_family=` registers fonts from Python and remaps the CSS generic families (`sans-serif` etc.) — verified rendering with a non-default font. This is the answer for fontless Alpine containers. |
| **The exact HA scenario** | **Verified end-to-end in Docker**: `maturin build --compatibility musllinux_1_2` inside `rust:alpine` (aarch64) → 6.5MB abi3 wheel → `pip install` in bare `python:3.12-alpine` → renders correctly with zero system fonts using Python-supplied font bytes. |
| Fontconfig portability trap | On Linux, Blitz's default `system-fonts` feature links libfontconfig (forbidden in manylinux/musllinux wheels; broke the first Alpine build). Solved by enabling fontique's `fontconfig-dlopen` feature: fontconfig is loaded at runtime *if present*, never linked — glibc desktops get system fonts, Alpine falls back to bundled fonts. |
| Python API cost | GIL released during render (`py.detach`), so it won't block Home Assistant's event loop when run in an executor. `render_rgba` returns raw pixels for zero-copy-ish handoff to Pillow (`Image.frombytes` → JPEG for the ESP display). |

The binding surface is intentionally tiny — two functions:

```python
render_png(html, *, width, height, scale=1.0, color_scheme="light"|"dark",
           background="#ffffff", base_url=None, fonts=None,
           default_font_family=None, allow_file_urls=False) -> bytes
render_rgba(...) -> (width, height, rgba_bytes)   # → PIL.Image.frombytes("RGBA", ...)
```

## Recommended architecture

**Bind the crates directly; keep the wrapper thin.**

- Depend on published `blitz-*` crates, not git `main` — beta.1 is only ~4 weeks behind main and pins us to reproducible builds. Prior art [hyper-render](https://github.com/thomasmost/hyper-render) validates the same recipe but is a stale single-maintainer wrapper; use as reference only.
- **Version pinning matters**: `blitz-paint 0.3.0-beta.1` needs `anyrender 0.11` + `anyrender_vello_cpu 0.14` (latest 0.15 is incompatible). The lockfile handles it; document it.
- No `blitz-net`, no tokio, no reqwest: rendering is deterministic and offline. Our 25-line `SyncProvider` handles `data:` URIs inline. If someone needs http assets later, it's a feature-gated add, not a rewrite.
- Skip JS forever; skip interactivity (that's `dioxus-native`'s job).

## Packaging plan (the "installs fast/cleanly" part)

- **abi3-py310 wheels** (already working): one wheel per platform covers Python 3.10+, including future versions. maturin builds these by default from our `pyo3` feature flags.
- **CI with [`maturin-action`](https://github.com/PyO3/maturin-action)**, wheel matrix:
  - `manylinux_2_28`: x86_64, aarch64
  - `musllinux_1_2`: x86_64, aarch64 ← **required for Home Assistant** (Alpine-based container)
  - macOS: arm64 (+ x86_64 if cheap)
  - Windows: x64 (free with maturin-action; widens the audience beyond our use case)
  - No armv7: HA [dropped 32-bit](https://www.home-assistant.io/blog/2025/05/22/deprecating-core-and-supervised-installation-methods-and-32-bit-systems/) as of 2025.12/2026.03. One less cross-compile headache.
- Wheel size ~4–6MB compressed (11MB dylib, thin LTO). Fine.
- For geekmagic-hacs: `manifest.json` gains one requirement line. Install becomes `pip install blitz-py` — no compiler on user machines, ever.

### Naming

`blitz` is taken on PyPI; **`blitz-py` / import `blitz_py`** (matching this repo) is free, as are `pyblitz`, `blitz-html`, `blitz-render`. Recommend keeping `blitz-py`.

### Font strategy (important for HA)

HA's container has ~no fonts. Recommendation: **bundle one compact open font** (e.g. Inter or Noto Sans, OFL-licensed) as package data, registered automatically as the default when no system font matches — plus the `fonts=[...]` escape hatch (already implemented) for user fonts. This makes output *identical across platforms*, which is a feature in itself for snapshot-testing widget layouts.

## Risks & caveats

1. **Blitz is pre-1.0 and churns.** Beta APIs changed under us once already during the spike (`anyrender` 0.11→0.12 split). Mitigation: exact-pin in `Cargo.lock`, upgrade deliberately, keep the binding surface small (2 functions = small blast radius).
2. **No `@font-face`, no external images** in the current design — by choice. Data URIs + `fonts=` cover the widget use case.
3. **Rendering fidelity gaps** exist vs a real browser (Blitz's own README: capable but buggy). For self-authored widget HTML this is fine; for arbitrary web pages it's not a goal.
4. **macOS 27 beta local-dev quirk** (not a product risk): the current beta's dyld requires 8-byte-aligned LINKEDIT string pools; today's Rust/Xcode linkers emit 4-aligned ones for *any* large dylib, so locally-built modules need a 30-line post-link realign script (in `scratchpad/align_strpool.py`, works). CI wheels built on stable runners are unaffected; upstream will fix before the OS GAs. Also: uv's python-build-standalone 3.12 crashes loading *any* pyo3 module on this beta — use Homebrew Python locally.

## Suggested roadmap

1. **v0.1.0** (≈ a day): tidy this spike — bundle default font, error handling (invalid HTML never panics across FFI), `pytest` suite with snapshot renders, README with geekmagic-style example, GitHub Actions wheel matrix, publish to PyPI.
2. **v0.2**: auto-height mode (`height=None` → content height, Blitz already computes it), `transparent` background shorthand, maybe markdown convenience (`render_markdown`, Blitz has a frontend for it).
3. **geekmagic-hacs integration**: new `HtmlRenderer` alongside the Pillow renderer; port one widget as proof; keep both paths during migration; run renders via `hass.async_add_executor_job` (GIL already released Rust-side).

