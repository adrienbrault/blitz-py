"""Smoke check for hosts where system-font enumeration fails or is empty.

Run on a container with fontconfig installed but zero fonts (fresh Home
Assistant base images): blitz-py must fall back to the bundled font and
still measure and render text, not raise pyo3_runtime.PanicException.
"""

import blitz_py

w, _ = blitz_py.measure_text("hello", font_size=16)
assert w > 0, "measure_text returned zero width"

_, _, rgba = blitz_py.render_rgba(
    '<body style="background:#fff;color:#000;font-size:32px">hello</body>',
    width=200,
    height=60,
)
assert any(rgba[i] < 128 for i in range(0, len(rgba), 4)), "render is blank"
print("fontless host: fallback OK, text rendered")
