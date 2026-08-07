"""Render a fixed set of documents and print content hashes.

CI runs this on every platform and asserts the hashes are identical —
the bundled font and CPU rasterizer make output byte-reproducible.
"""

import hashlib
import json
import sys

import blitz_py

GOLDENS = {
    "text": (
        "<style>body{margin:0;background:#fff;color:#123;font-family:sans-serif}</style>"
        "<body><h1>Heading 1</h1><p>Body text with <b>bold</b>, <i>italic</i> and "
        "<code>code</code>. 0123456789 — “quotes” &amp; ligatures: ffi ffl.</p>",
        400,
        200,
        1.0,
    ),
    "layout": (
        "<style>body{margin:0}</style>"
        "<body><div style='display:flex;gap:8px;padding:8px;background:#eef'>"
        "<div style='flex:1;height:60px;background:linear-gradient(#f00,#00f);border-radius:8px'></div>"
        "<div style='width:40%;height:60px;background:#0a84ff33;border:2px dashed #345'></div>"
        "</div><div style='display:grid;grid-template-columns:1fr 2fr;gap:4px;padding:8px'>"
        "<div style='height:30px;background:#30d158'></div>"
        "<div style='height:30px;background:#ffd60a;transform:rotate(2deg)'></div></div>",
        300,
        140,
        2.0,
    ),
    "widget": (
        "<style>body{margin:0;width:240px;height:240px;background:#000;color:#fff;"
        "font-family:sans-serif}.t{display:flex;align-items:center;justify-content:center;"
        "height:100%;font-size:64px;font-weight:600}</style>"
        "<body><div class='t'>21.5&deg;</div>",
        240,
        240,
        1.0,
    ),
    "animation_frame": (
        "<style>body{margin:0;background:#fff}"
        ".b{width:40px;height:40px;background:#e11;animation:m 2s linear infinite}"
        "@keyframes m{from{transform:translateX(0) rotate(0)}to{transform:translateX(150px) rotate(180deg)}}"
        "</style><body><div class='b'></div>",
        220,
        60,
        1.0,
    ),
}


def main() -> int:
    hashes = {}
    for name, (html, w, h, scale) in GOLDENS.items():
        if name == "animation_frame":
            _, _, frames = blitz_py.render_frames(
                html, width=w, height=h, scale=scale, times=[0.75]
            )
            data = frames[0]
        else:
            _, _, data = blitz_py.render_rgba(html, width=w, height=h, scale=scale)
        hashes[name] = hashlib.sha256(data).hexdigest()
    print(json.dumps(hashes, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
