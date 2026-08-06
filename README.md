# blitz-py

Render HTML/CSS to images from Python — no browser, no GPU, no JavaScript, no network.

Powered by [Blitz](https://github.com/DioxusLabs/blitz), DioxusLabs' modular web engine: real CSS via [Stylo](https://github.com/servo/stylo) (Servo/Firefox's style engine), flexbox/grid layout via [Taffy](https://github.com/DioxusLabs/taffy), text shaping via [Parley](https://github.com/linebender/parley), and CPU rasterization via [vello_cpu](https://github.com/linebender/vello).

A 240×240 widget renders in ~20ms on an M-series Mac. Output is deterministic and identical across platforms: the [Inter](https://rsms.me/inter/) font (SIL OFL 1.1) is bundled as the default face, so text renders the same on your laptop and in a fontless Alpine container.

## Install

```
pip install blitz-py
```

Prebuilt wheels (abi3, Python ≥ 3.10): Linux glibc + musl (x86_64, aarch64), macOS (arm64, x86_64), Windows (x64).

## Usage

```python
import blitz_py

png = blitz_py.render_png(
    """
    <style>
      body { margin: 0; background: #000; color: #fff; font-family: sans-serif; }
      .screen { display: flex; flex-direction: column; align-items: center;
                justify-content: center; height: 240px; }
      .temp { font-size: 64px; font-weight: 600; }
      .label { color: #8e8e93; }
    </style>
    <body><div class="screen">
      <div class="temp">21.5&deg;</div>
      <div class="label">Living room</div>
    </div></body>
    """,
    width=240,
    height=240,
)
with open("out.png", "wb") as f:
    f.write(png)
```

Or get raw pixels for [Pillow](https://pillow.readthedocs.io/):

```python
from PIL import Image

w, h, rgba = blitz_py.render_rgba(html, width=240, height=240)
Image.frombytes("RGBA", (w, h), rgba).convert("RGB").save("out.jpg", quality=90)
```

## API

Two functions, same keyword arguments:

```python
render_png(html, *, width, height, ...) -> bytes            # PNG file bytes
render_rgba(html, *, width, height, ...) -> (w, h, bytes)   # raw RGBA pixels
```

| Argument | Default | Meaning |
|---|---|---|
| `width`, `height` | required | CSS-pixel viewport size |
| `scale` | `1.0` | Device-pixel ratio; output is `width*scale` × `height*scale` physical pixels. Use `2.0` for supersampled/hi-dpi output. |
| `color_scheme` | `"light"` | `"light"` or `"dark"` — drives `@media (prefers-color-scheme: ...)` |
| `background` | `"#ffffff"` | Base canvas color (`#rgb`, `#rrggbb`, `#rrggbbaa`), or `None` for transparent |
| `base_url` | `None` | Base for resolving relative URLs |
| `fonts` | `None` | List of font file `bytes` (TTF/OTF, variable fonts OK) to register |
| `default_font_family` | `None` | Family name to use for all CSS generic families (`sans-serif`, `serif`, ...) and as text fallback |
| `allow_file_urls` | `False` | Permit `file://` URLs for images/resources |

### Images and resources

Rendering is fully offline. Embed images as `data:` URIs, or enable `allow_file_urls=True` and use `file://` paths. `http(s)` URLs are intentionally ignored.

```python
import base64
b64 = base64.b64encode(open("icon.png", "rb").read()).decode()
html = f'<img src="data:image/png;base64,{b64}" style="width:32px">'
```

### Fonts

Bundled Inter is the default for every CSS generic family and the Latin-script fallback, everywhere. Explicit family names (`font-family: "Comic Sans MS"`) resolve against system fonts where available (macOS/Windows natively; Linux via fontconfig loaded at runtime if present — never a link dependency). To use your own font:

```python
font = open("MyFont.ttf", "rb").read()
blitz_py.render_png(html, width=240, height=240,
                    fonts=[font], default_font_family="My Font")
```

### What's supported

Modern CSS as implemented by Stylo/Taffy: flexbox, grid, gradients, border-radius, shadows, transforms, `calc()`, custom properties, media queries, SVG images, WOFF... No JavaScript, no `@font-face` fetching, no external resources. Blitz itself is pre-1.0: capable but not pixel-perfect against browsers.

## Why not a headless browser?

Playwright/Chromium render HTML too — at ~150MB+ of install, a browser process to babysit, and cold starts in the hundreds of milliseconds. blitz-py is a ~7MB self-contained wheel with millisecond renders, suitable for embedded targets like Home Assistant integrations generating widget images for small displays (its original use case).

## License

MIT OR Apache-2.0. Bundled Inter font: SIL OFL 1.1 ([assets/LICENSE-Inter.txt](assets/LICENSE-Inter.txt)).
