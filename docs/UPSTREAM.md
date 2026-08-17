# Upstream findings

Engine behaviors discovered while building blitz-py. First found against `blitz 0.3.0-beta.1` / `fontique 0.10` / `vello_cpu 0.0.9`; last re-checked 2026-08-17 against `blitz main @ 01a6c4b` (git pin, post-beta.1) / `fontique 0.11.1` / `vello_cpu 0.1.0`. Kept here as the source of truth for our workarounds; revisit on every dependency bump.

| Finding | Where | Our workaround | Status upstream |
|---|---|---|---|
| `parley::FontContext::new()` leaks ~120KB per call (platform font scan) and costs ~25ms | fontique 0.10, still present in 0.11.1 | Build the font collection once, clone per render (`base_collection()`) | Not reported (repro: `examples/leak.rs`, stages `fontctx` vs `paint`) |
| `@font-face` sources with no recognizable `format()` hint and no URL file extension are silently skipped (data: URIs) | blitz-dom `fetch_font_face` | Documented in README: use `format(truetype)` / `format("ttf")` | Related fix landed on main (panic → skip, #616) but the silent-skip remains |
| SVG `<text>` resolves fonts via usvg's own fontdb, not the document font collection — bundled/registered fonts don't apply | blitz-paint / anyrender_svg | Avoid SVG text; overlay HTML text | Not reported |
| CSS *animations* don't apply to `<svg>` elements (static transforms do) | blitz-dom style/animation | Animate a wrapping `<div>` | Not reported |
| `DocumentMutator::set_style_property` doesn't invalidate layout | blitz-dom 0.3.0-beta.1 | `Template.set_style` rewrites the `style` attribute instead (kept — equally correct, and no need to churn) | Fixed on main (#582); included in our git pin |
| Bundled font family naming: fontique registers fonts under internal name (`"Inter Variable"`), so name-based defaults can silently bind to a system font or nothing | fontique | Register with `FontInfoOverride { family_name }` and wire generics by `FamilyId` | Working as designed; gotcha documented |
| System-font scan panics when fontconfig is installed but zero fonts are (`font_sort(...).unwrap()` on `NoMatch` while populating generic families) — hit on fresh Home Assistant base images | fontique 0.10 `backend/fontconfig.rs` (still `unwrap()`s in 0.11.1) | Probe `Collection::new(system_fonts: true)` under `catch_unwind`; fall back to bundled-fonts-only (`base_mutex()`) | Fixed on parley main (`let Ok(..) else` skip), in no release ≤ 0.11.1 — drop the workaround at the next parley/blitz bump |

Worth contributing upstream when we engage: `text-overflow: ellipsis` support in blitz-dom — currently consumers re-implement truncation in application code; `measure_text` (v0.3.0) removes the metric-mismatch pain but native ellipsis is the real fix.

Also worth noting: `anyrender` 0.11 fails to compile on 32-bit targets — `const _: [u8; 128] = [0; size_of::<FilterEffect>()]` assumes 64-bit pointer sizes (88 bytes on armv7). This blocks armv7l wheels entirely.

Also relevant but not engine bugs:

- The `anyrender` / `anyrender_vello_cpu` / `parley` versions must match what the pinned blitz commit uses (blitz's workspace `Cargo.toml`): beta.1 needed `anyrender 0.11` + `anyrender_vello_cpu 0.14`; main @ 01a6c4b needs `anyrender 0.13` + `anyrender_vello_cpu 0.16` + `parley 0.11`. Re-pair on every bump.
- Since 2026-08-17 we depend on blitz `main` via a git `rev` pin (no post-beta.1 release exists yet, and main carries ~140 layout/robustness fixes we want, e.g. #636/#695 resource gating with a sync `NetProvider`, #655/#616/#648 panic fixes, table/float/replaced-element sizing, `background-attachment: fixed`, SVG background sizing). Switch back to a crates.io version at the next blitz release.
- macOS 26/27-beta local-dev quirks (dyld LINKEDIT alignment; see `tools/align_strpool.py`) are host-OS issues, not Blitz.
