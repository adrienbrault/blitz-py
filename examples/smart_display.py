"""A 240x240 smart-display frame in the style of iOS StandBy:

- cells are `Template`s (parse once, `update()` fresh data each refresh)
- one `render_layers_png` call composites clock + metrics + media + glow
- `fit_font_size` autoscales the clock, `ellipsize` fits the track title
- rendered at scale=2 for a crisp README image
"""

import blitz_py

S = 2.0  # supersample factor
INK = "#f4f6fb"
MUTED = "#77809266"
LABEL = "#8a93a6"
ACCENT = "#5ac8fa"

BASE = (
    "<style>*{margin:0;box-sizing:border-box}"
    "body{width:240px;height:%dpx;font-family:sans-serif}"
    ".label{font-size:9px;letter-spacing:0.22em;text-transform:uppercase;color:%s}"
    "</style>"
)


def cell(html: str, h: int) -> blitz_py.Template:
    return blitz_py.Template(
        (BASE % (h, LABEL)) + html, width=240, height=h, scale=S, background=None
    )


# --- clock: dominant, tight tracking, autoscaled ---------------------------
clock_size = blitz_py.fit_font_size("21:47", max_width=196.0, max_size=88.0, font_weight=650)
clock = cell(
    "<body><div style='height:100%;display:flex;flex-direction:column;"
    "align-items:center;justify-content:flex-end;gap:2px'>"
    "<p class='label'>Salon</p>"
    f"<span id='t' style='font-size:{clock_size:.0f}px;font-weight:650;"
    f"letter-spacing:-0.045em;color:{INK};line-height:1'>21:47</span>"
    "</div></body>",
    120,
)

# --- metrics: three complications, value over hairline label ---------------
metric = (
    "<div style='display:flex;flex-direction:column;align-items:center;gap:5px'>"
    f"<span id='{{id}}' style='font-size:21px;font-weight:600;color:{INK};"
    "letter-spacing:-0.01em'>{value}</span>"
    "<p class='label'>{label}</p></div>"
)
metrics = cell(
    "<body><div style='height:100%;display:flex;align-items:center;"
    "justify-content:space-between;padding:0 30px'>"
    + metric.format(id="temp", value="21.5°", label="Temp")
    + metric.format(id="hum", value="48%", label="Hum")
    + metric.format(id="co2", value="612", label="CO&#8322;")
    + "</div></body>",
    64,
)

# --- now playing: one quiet line -------------------------------------------
title = blitz_py.ellipsize(
    "Daft Punk — Harder, Better, Faster, Stronger",
    max_width=180.0,
    font_size=11.0,
    font_weight=500,
)
media = cell(
    "<body><div style='height:100%;display:flex;align-items:center;"
    "justify-content:center;gap:7px'>"
    f"<span style='color:{ACCENT};font-size:10px'>&#9835;</span>"
    f"<span id='song' style='font-size:11px;font-weight:500;color:#aeb6c4'>{title}</span>"
    "</div></body>",
    36,
)

# --- fresh data, as a refresh loop would do --------------------------------
clock.update(t="21:47")
metrics.update(temp="21.5°", hum="48%", co2="612")

# --- a whisper of glow behind the clock ------------------------------------
GLOW = (
    "<style>*{margin:0}body{width:240px;height:240px}</style>"
    "<body><div style='position:absolute;left:52px;top:56px;width:136px;height:56px;"
    "border-radius:50%;background:#5ac8fa'></div></body>"
)

px = lambda v: int(v * S)  # noqa: E731

frame = blitz_py.render_layers_png(
    [
        {"html": GLOW, "width": 240, "height": 240, "scale": S, "blur": 26.0,
         "tint": ACCENT, "opacity": 0.22},
        {"template": clock, "x": 0, "y": px(22)},
        {"template": metrics, "x": 0, "y": px(150)},
        {"template": media, "x": 0, "y": px(198)},
    ],
    width=px(240),
    height=px(240),
    background="#000000",
)
with open("docs/samples/sample_display.png", "wb") as f:
    f.write(frame)
print(f"docs/samples/sample_display.png: {len(frame) // 1024}KB")
