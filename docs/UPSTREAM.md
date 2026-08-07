# Upstream findings

Engine behaviors discovered while building blitz-py, against `blitz 0.3.0-beta.1` / `fontique 0.10` / `vello_cpu 0.0.9`. Kept here as the source of truth for our workarounds; revisit on every dependency bump.

| Finding | Where | Our workaround | Status upstream |
|---|---|---|---|
| `parley::FontContext::new()` leaks ~120KB per call (platform font scan) and costs ~25ms | fontique 0.10 | Build the font collection once, clone per render (`base_collection()`) | Not reported (repro: `examples/leak.rs`, stages `fontctx` vs `paint`) |
| `@font-face` sources with no recognizable `format()` hint and no URL file extension are silently skipped (data: URIs) | blitz-dom `fetch_font_face` | Documented in README: use `format(truetype)` / `format("ttf")` | Related fix landed on main (panic → skip, #616) but the silent-skip remains |
| SVG `<text>` resolves fonts via usvg's own fontdb, not the document font collection — bundled/registered fonts don't apply | blitz-paint / anyrender_svg | Avoid SVG text; overlay HTML text | Not reported |
| CSS *animations* don't apply to `<svg>` elements (static transforms do) | blitz-dom style/animation | Animate a wrapping `<div>` | Not reported |
| `DocumentMutator::set_style_property` doesn't invalidate layout | blitz-dom 0.3.0-beta.1 | `Template.set_style` rewrites the `style` attribute instead | Fixed on main (#582), unreleased |
| Bundled font family naming: fontique registers fonts under internal name (`"Inter Variable"`), so name-based defaults can silently bind to a system font or nothing | fontique | Register with `FontInfoOverride { family_name }` and wire generics by `FamilyId` | Working as designed; gotcha documented |

Worth contributing upstream when we engage: `text-overflow: ellipsis` support in blitz-dom — currently consumers re-implement truncation in application code; `measure_text` (v0.3.0) removes the metric-mismatch pain but native ellipsis is the real fix.

Also relevant but not engine bugs:

- `anyrender_vello_cpu` 0.15 requires `anyrender` 0.12, incompatible with `blitz-paint 0.3.0-beta.1` (needs 0.11) — pin 0.14 until the next blitz release.
- macOS 26/27-beta local-dev quirks (dyld LINKEDIT alignment; see `tools/align_strpool.py`) are host-OS issues, not Blitz.
