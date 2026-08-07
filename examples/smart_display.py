"""A 240x240 smart-display frame composed with the v0.4 toolkit:

- cells are `Template`s (parse once, `update()` with fresh data each refresh)
- one `render_layers_png` call composites backdrop + cells + glow accents
- `ellipsize` fits the track title, `fit_font_size` autoscales the clock
- rendered at scale=2 for a crisp README image
"""

import blitz_py

S = 2.0  # supersample factor


def cell(html: str, w: int, h: int) -> blitz_py.Template:
    base = (
        "<style>*{margin:0;box-sizing:border-box}"
        f"body{{width:{w}px;height:{h}px;font-family:sans-serif;color:#fff}}"
        ".card{height:100%;background:#ffffff0d;border:1px solid #ffffff1a;"
        "border-radius:14px;padding:10px 12px;display:flex;flex-direction:column}"
        ".label{font-size:10px;letter-spacing:0.14em;text-transform:uppercase;color:#7d8aa0}"
        "</style>"
    )
    return blitz_py.Template(base + html, width=w, height=h, scale=S, background=None)


# --- cells -----------------------------------------------------------------
clock = cell(
    "<body><div class='card'><p class='label'>Time</p>"
    "<div style='flex:1;display:flex;align-items:center;justify-content:center'>"
    f"<span id='t' style='font-size:{blitz_py.fit_font_size('21:47', max_width=180, max_size=52):.0f}px;"
    "font-weight:700;letter-spacing:-0.02em'>21:47</span></div></div></body>",
    208,
    84,
)

temp = cell(
    "<body><div class='card'><p class='label'>Salon</p>"
    "<div style='flex:1;display:flex;align-items:baseline;justify-content:center;gap:4px'>"
    "<span id='v' style='font-size:34px;font-weight:700'>21.5°</span>"
    "<span style='font-size:12px;color:#30d158'>&#8593;</span></div></div></body>",
    100,
    72,
)

hum = cell(
    "<body><div class='card'><p class='label'>Humidity</p>"
    "<div style='flex:1;display:flex;flex-direction:column;justify-content:center;gap:6px'>"
    "<span id='v' style='font-size:22px;font-weight:600'>48%</span>"
    "<div style='height:5px;border-radius:99px;background:#ffffff14'>"
    "<div id='bar' style='height:5px;width:48%;border-radius:99px;"
    "background:linear-gradient(90deg,#00d9ff,#6366f1)'></div></div></div></div></body>",
    100,
    72,
)

title = blitz_py.ellipsize(
    "Daft Punk — Harder, Better, Faster, Stronger", max_width=136.0, font_size=13, font_weight=600
)
media = cell(
    "<body><div class='card' style='flex-direction:row;align-items:center;gap:10px'>"
    "<div style='width:28px;height:28px;border-radius:8px;flex-shrink:0;"
    "background:linear-gradient(135deg,#00d9ff,#6366f1);display:flex;align-items:center;"
    "justify-content:center;font-size:14px'>&#9835;</div>"
    f"<div style='min-width:0'><p id='song' style='font-size:13px;font-weight:600'>{title}</p>"
    "<p class='label' style='margin-top:2px'>now playing</p></div></div></body>",
    208,
    46,
)

# --- fresh data, as a refresh loop would do --------------------------------
clock.update(t="21:47")
temp.update(v="21.5°")
hum.update(v="48%")
hum.set_style("bar", "width", "48%")

# --- glow accent: the gradient bits re-rendered blurred under everything ---
ACCENT = (
    "<style>*{margin:0}body{width:240px;height:240px}</style>"
    "<body><div style='position:absolute;left:24px;top:190px;width:34px;height:34px;"
    "border-radius:9px;background:linear-gradient(135deg,#00d9ff,#6366f1)'></div></body>"
)

BACKDROP = (
    "<style>*{margin:0}body{width:240px;height:240px;"
    "background:radial-gradient(circle at 30% 20%, #17203a, #0a0e1c 70%)}</style><body></body>"
)

px = lambda v: int(v * S)  # noqa: E731

frame = blitz_py.render_layers_png(
    [
        {"html": BACKDROP, "width": 240, "height": 240, "scale": S},
        {"html": ACCENT, "width": 240, "height": 240, "scale": S, "blur": 14.0,
         "tint": "#00d9ff", "opacity": 0.8},
        {"template": clock, "x": px(16), "y": px(12)},
        {"template": temp, "x": px(16), "y": px(102)},
        {"template": hum, "x": px(124), "y": px(102)},
        {"template": media, "x": px(16), "y": px(182)},
    ],
    width=px(240),
    height=px(240),
    background="#0a0e1c",
)
with open("docs/samples/sample_display.png", "wb") as f:
    f.write(frame)
print(f"docs/samples/sample_display.png: {len(frame) // 1024}KB")
