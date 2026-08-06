"""Render a smart-display-style 240x240 widget and benchmark it."""

import base64
import io
import time

import blitz_py

# Tiny red 8x8 PNG as a data URI (stand-in for a weather icon)
try:
    from PIL import Image

    buf = io.BytesIO()
    Image.new("RGBA", (8, 8), (255, 80, 80, 255)).save(buf, format="PNG")
    icon_b64 = base64.b64encode(buf.getvalue()).decode()
    HAVE_PIL = True
except ImportError:
    icon_b64 = ""
    HAVE_PIL = False

HTML = f"""
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{ width: 240px; height: 240px; background: #000; color: #fff;
         font-family: sans-serif; }}
  .screen {{ display: flex; flex-direction: column; height: 100%; padding: 12px; }}
  .header {{ display: flex; justify-content: space-between; align-items: center;
             color: #8e8e93; font-size: 14px; }}
  .temp {{ flex: 1; display: flex; align-items: center; justify-content: center;
           font-size: 64px; font-weight: 600;
           background: linear-gradient(180deg, #0a84ff22, transparent);
           border-radius: 16px; margin: 8px 0; }}
  .footer {{ display: flex; justify-content: space-between; font-size: 13px; }}
  .pill {{ background: #1c1c1e; border-radius: 999px; padding: 4px 10px; }}
  .accent {{ color: #0a84ff; }}
  img {{ width: 16px; height: 16px; }}
</style>
<body>
  <div class="screen">
    <div class="header">
      <span>Salon</span>
      <img src="data:image/png;base64,{icon_b64}">
      <span>21:34</span>
    </div>
    <div class="temp">21.5&deg;</div>
    <div class="footer">
      <span class="pill">HUM <span class="accent">48%</span></span>
      <span class="pill">CO2 <span class="accent">612</span></span>
    </div>
  </div>
</body>
"""

# Warmup + timed runs
t0 = time.perf_counter()
png = blitz_py.render_png(HTML, width=240, height=240)
t1 = time.perf_counter()

N = 10
t2 = time.perf_counter()
for _ in range(N):
    blitz_py.render_png(HTML, width=240, height=240)
t3 = time.perf_counter()

with open("examples/out_widget.png", "wb") as f:
    f.write(png)

# 2x supersampled render (crisper text on small displays)
png2x = blitz_py.render_png(HTML, width=240, height=240, scale=2.0)
with open("examples/out_widget_2x.png", "wb") as f:
    f.write(png2x)

# Raw RGBA -> PIL -> JPEG (the geekmagic path)
if HAVE_PIL:
    w, h, rgba = blitz_py.render_rgba(HTML, width=240, height=240)
    img = Image.frombytes("RGBA", (w, h), rgba).convert("RGB")
    img.save("examples/out_widget.jpg", quality=90)

print(f"first render (cold): {(t1 - t0) * 1000:.1f}ms")
print(f"avg of {N} renders:   {(t3 - t2) / N * 1000:.1f}ms")
print(f"png size: {len(png)} bytes, 2x: {len(png2x)} bytes")
