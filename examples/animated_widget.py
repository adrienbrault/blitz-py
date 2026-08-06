"""Render a CSS-animated 240x240 widget to a looping GIF."""

from PIL import Image

import blitz_py

HTML = """
<style>
  * { margin: 0; box-sizing: border-box; }
  body { width: 240px; height: 240px; background: #000; color: #fff; font-family: sans-serif; }
  .screen { display: flex; flex-direction: column; height: 100%; padding: 14px; }
  .header { display: flex; align-items: center; justify-content: space-between;
            color: #8e8e93; font-size: 13px; }
  .live { display: flex; align-items: center; gap: 5px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: #30d158;
         animation: pulse 1.6s ease-in-out infinite; }
  @keyframes pulse { 0%, 100% { opacity: 1; transform: scale(1); }
                     50% { opacity: 0.35; transform: scale(0.7); } }
  .temp { flex: 1; display: flex; align-items: center; justify-content: center;
          font-size: 60px; font-weight: 600; border-radius: 16px; margin: 10px 0;
          background: linear-gradient(180deg, #0a84ff26, transparent); }
  .bar { height: 6px; border-radius: 999px; background: #1c1c1e; overflow: hidden; }
  .fill { height: 100%; width: 62%; border-radius: 999px;
          background: linear-gradient(90deg, #0a84ff, #64d2ff);
          transform-origin: left; animation: grow 3.2s ease-in-out infinite; }
  @keyframes grow { 0%, 100% { transform: scaleX(0.55); } 50% { transform: scaleX(1.0); } }
  .footer { display: flex; justify-content: space-between; font-size: 12px;
            color: #8e8e93; margin-top: 8px; }
  .up { color: #30d158; animation: nudge 1.6s ease-in-out infinite; display: inline-block; }
  @keyframes nudge { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-3px); } }
</style>
<body><div class="screen">
  <div class="header"><span>Salon</span><span class="live"><span class="dot"></span>live</span></div>
  <div class="temp">21.5&deg;</div>
  <div class="bar"><div class="fill"></div></div>
  <div class="footer"><span>ventilation</span><span><span class="up">&#8593;</span> auto</span></div>
</div></body>
"""

FPS, SECONDS = 12, 3.2  # 3.2s = one loop of the slowest animation

w, h, frames = blitz_py.render_frames(
    HTML, width=240, height=240, times=[i / FPS for i in range(int(FPS * SECONDS))]
)
rgbs = [Image.frombytes("RGBA", (w, h), f).convert("RGB") for f in frames]

# One shared palette and no dithering: dither noise and per-frame palettes
# both defeat GIF's LZW/delta compression (this widget: 163KB naive vs 18KB).
base = rgbs[0].quantize(colors=64, dither=Image.Dither.NONE)
imgs = [im.quantize(colors=64, palette=base, dither=Image.Dither.NONE) for im in rgbs]

imgs[0].save(
    "examples/out_widget.gif",
    save_all=True,
    append_images=imgs[1:],
    duration=int(1000 / FPS),
    loop=0,
    optimize=True,
)
print(f"examples/out_widget.gif: {len(imgs)} frames at {w}x{h}")
