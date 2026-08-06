# blitz-py

Render HTML/CSS to images from Python — no browser, no GPU, no JavaScript.

Powered by [Blitz](https://github.com/DioxusLabs/blitz), DioxusLabs' modular web engine: real CSS via [Stylo](https://github.com/servo/stylo) (Servo/Firefox's style engine), flexbox/grid layout via [Taffy](https://github.com/DioxusLabs/taffy), text shaping via [Parley](https://github.com/linebender/parley), and CPU rasterization via [vello_cpu](https://github.com/linebender/vello).

```python
import blitz_py

png = blitz_py.render_png(
    """
    <div style="display: flex; align-items: center; justify-content: center;
                height: 100%; background: #111; color: #fff; font-family: sans-serif;">
      <h1>Hello from CSS</h1>
    </div>
    """,
    width=240,
    height=240,
)
with open("out.png", "wb") as f:
    f.write(png)
```

## API

- `render_png(html, *, width, height, scale=1.0, color_scheme="light", background="#ffffff", base_url=None, fonts=None, default_font_family=None, allow_file_urls=False) -> bytes`
- `render_rgba(...) -> (width, height, bytes)` — raw RGBA pixels, ready for `PIL.Image.frombytes("RGBA", (w, h), data)`.

Images can be embedded as `data:` URIs. Network fetching is intentionally not supported — rendering is deterministic and offline. Pass font bytes via `fonts=[...]` to bundle your own fonts (e.g. for minimal containers with no system fonts).

## Status

Experimental. Built on pre-1.0 Blitz crates.
