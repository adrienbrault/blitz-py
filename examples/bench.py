"""Benchmark blitz-py: timing + memory per scenario, each in a fresh process.

Usage: python examples/bench.py            # all scenarios, markdown table
       python examples/bench.py <name>     # one scenario, JSON line

The bootstrap scenario downloads bootstrap.min.css on first run (cached next
to this script). Numbers in the README were produced by this script.
"""

import json
import pathlib
import resource
import subprocess
import sys
import time
import urllib.request

HERE = pathlib.Path(__file__).parent

WIDGET_HTML = """
<style>
  * { margin: 0; box-sizing: border-box; }
  body { width: 240px; height: 240px; background: #000; color: #fff; font-family: sans-serif; }
  .screen { display: flex; flex-direction: column; height: 100%; padding: 12px; }
  .header { display: flex; justify-content: space-between; color: #8e8e93; font-size: 14px; }
  .temp { flex: 1; display: flex; align-items: center; justify-content: center;
          font-size: 64px; font-weight: 600;
          background: linear-gradient(180deg, #0a84ff22, transparent);
          border-radius: 16px; margin: 8px 0; }
  .footer { display: flex; justify-content: space-between; font-size: 13px; }
  .pill { background: #1c1c1e; border-radius: 999px; padding: 4px 10px; }
  .accent { color: #0a84ff; }
</style>
<body><div class="screen">
  <div class="header"><span>Salon</span><span>21:34</span></div>
  <div class="temp">21.5&deg;</div>
  <div class="footer">
    <span class="pill">HUM <span class="accent">48%</span></span>
    <span class="pill">CO2 <span class="accent">612</span></span>
  </div>
</div></body>"""

BOOTSTRAP_BODY = """
<body class="p-4 bg-light">
  <div class="card shadow-sm" style="width: 24rem">
    <div class="card-body">
      <h5 class="card-title">Bootstrap card</h5>
      <h6 class="card-subtitle mb-2 text-body-secondary">Rendered by blitz-py</h6>
      <p class="card-text">Real Bootstrap 5.3 CSS, inlined. Buttons, badges, alerts:</p>
      <button class="btn btn-primary me-2">Primary</button>
      <button class="btn btn-outline-danger">Outline</button>
      <div class="alert alert-success mt-3 mb-0">It works <span class="badge text-bg-warning">new</span></div>
    </div>
  </div>
  <div class="progress mt-3" style="width: 24rem">
    <div class="progress-bar bg-info" style="width: 65%">65%</div>
  </div>
</body>"""

ARTICLE_PARA = (
    "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod"
    " tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim"
    " veniam, quis nostrud exercitation ullamco.</p>"
)


def bootstrap_css() -> str:
    cache = HERE / ".bootstrap.min.css"
    if not cache.exists():
        url = "https://cdn.jsdelivr.net/npm/bootstrap@5.3.3/dist/css/bootstrap.min.css"
        cache.write_bytes(urllib.request.urlopen(url).read())
    return cache.read_text()


def scenario(name: str) -> tuple[str, int, int, float, str | None]:
    """Returns (html, width, height, scale, background)."""
    if name == "tiny":
        return "<h1>Hello, world</h1>", 200, 100, 1.0, "#ffffff"
    if name == "widget":
        return WIDGET_HTML, 240, 240, 2.0, "#ffffff"
    if name == "bootstrap":
        return f"<style>{bootstrap_css()}</style>{BOOTSTRAP_BODY}", 440, 360, 2.0, "#ffffff"
    if name == "tailwind":
        html = (HERE / "tailwind_dashboard.html").read_text()
        return html, 760, 664, 2.0, "#0b1120"
    if name == "article":
        html = (
            "<style>body{font-family:sans-serif;margin:40px;line-height:1.6}"
            "h2{color:#234}blockquote{border-left:4px solid #08f;padding-left:12px;color:#456}</style>"
            "<body><h1>Long article</h1>"
            + ("<h2>Section</h2>" + ARTICLE_PARA * 4 + "<blockquote>Pull quote.</blockquote>") * 10
            + "</body>"
        )
        return html, 800, 4000, 1.0, "#ffffff"
    raise SystemExit(f"unknown scenario {name}")


def run_gif(name: str) -> None:
    """Animated widget -> frames -> Pillow GIF, timing render and encode."""
    import io

    import blitz_py
    from PIL import Image

    html = (HERE / "animated_widget.py").read_text().split('HTML = """')[1].split('"""')[0]
    fps, seconds = 12, 3.2
    times = [i / fps for i in range(int(fps * seconds))]

    t0 = time.perf_counter()
    w, h, frames = blitz_py.render_frames(html, width=240, height=240, times=times)
    t_render = time.perf_counter() - t0

    t0 = time.perf_counter()
    rgbs = [Image.frombytes("RGBA", (w, h), f).convert("RGB") for f in frames]
    base = rgbs[0].quantize(colors=64, dither=Image.Dither.NONE)
    imgs = [im.quantize(colors=64, palette=base, dither=Image.Dither.NONE) for im in rgbs]
    buf = io.BytesIO()
    imgs[0].save(buf, format="GIF", save_all=True, append_images=imgs[1:],
                 duration=int(1000 / fps), loop=0, optimize=True)
    t_encode = time.perf_counter() - t0

    rss_peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform != "darwin":
        rss_peak *= 1024
    print(
        json.dumps(
            {
                "scenario": name,
                "px": f"{w}x{h} x{len(frames)} frames",
                "cold_ms": round(t_render * 1000, 1),
                "warm_ms": round(t_render / len(frames) * 1000, 2),
                "encode_ms": round(t_encode * 1000, 1),
                "png_kb": round(len(buf.getvalue()) / 1024, 1),
                "rss_peak_after_200_mb": round(rss_peak / 1e6, 1),
            }
        )
    )


def run_one(name: str) -> None:
    import blitz_py

    if name == "gif":
        return run_gif(name)

    html, w, h, scale, bg = scenario(name)

    t0 = time.perf_counter()
    png = blitz_py.render_png(html, width=w, height=h, scale=scale, background=bg)
    t_cold = time.perf_counter() - t0

    n = 50
    t0 = time.perf_counter()
    for _ in range(n):
        blitz_py.render_png(html, width=w, height=h, scale=scale, background=bg)
    t_warm = (time.perf_counter() - t0) / n

    for _ in range(150):
        blitz_py.render_png(html, width=w, height=h, scale=scale, background=bg)
    rss_peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform != "darwin":
        rss_peak *= 1024  # linux reports KB, macOS bytes

    print(
        json.dumps(
            {
                "scenario": name,
                "px": f"{int(w * scale)}x{int(h * scale)}",
                "cold_ms": round(t_cold * 1000, 1),
                "warm_ms": round(t_warm * 1000, 2),
                "png_kb": round(len(png) / 1024, 1),
                "rss_peak_after_200_mb": round(rss_peak / 1e6, 1),
            }
        )
    )


ALL = ["tiny", "widget", "bootstrap", "tailwind", "article", "gif"]

if __name__ == "__main__":
    if len(sys.argv) > 1:
        run_one(sys.argv[1])
    else:
        rows = []
        for s in ALL:
            r = subprocess.run([sys.executable, __file__, s], capture_output=True, text=True)
            out = r.stdout.strip()
            if not out:
                print(f"{s}: FAILED\n{r.stderr[-300:]}", file=sys.stderr)
                continue
            rows.append(json.loads(out))
        print("| Scenario | Output px | First render | Warm render | Peak RSS after 200 renders |")
        print("|---|---|---:|---:|---:|")
        for r in rows:
            print(
                f"| {r['scenario']} | {r['px']} | {r['cold_ms']:.0f}ms "
                f"| **{r['warm_ms']:.1f}ms** | {r['rss_peak_after_200_mb']:.0f}MB |"
            )
