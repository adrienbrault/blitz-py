"""v0.6.0: in-memory resources."""

import base64
import struct
import zlib

import pytest

import blitz_py


def png_bytes(w: int, h: int, rgb: tuple[int, int, int]) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    row = b"\x00" + bytes(rgb) * w
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(row * h))
        + chunk(b"IEND", b"")
    )


def pixel(rgba: bytes, w: int, x: int, y: int) -> tuple[int, int, int, int]:
    off = (y * w + x) * 4
    return tuple(rgba[off : off + 4])


RED = png_bytes(20, 10, (255, 0, 0))
IMG_DOC = "<style>body{margin:0} img{display:block}</style><img id=i src='%s'>"


class TestResources:
    @pytest.mark.parametrize("src", ["logo.png", "./logo.png", "/logo.png"])
    def test_relative_image(self, src):
        w, _, rgba = blitz_py.render_rgba(
            IMG_DOC % src, width=40, height=20, resources={"logo.png": RED}
        )
        assert pixel(rgba, w, 5, 5) == (255, 0, 0, 255)

    def test_missing_resource_renders_as_missing(self):
        w, _, rgba = blitz_py.render_rgba(
            IMG_DOC % "other.png", width=40, height=20, resources={"logo.png": RED}
        )
        assert pixel(rgba, w, 5, 5) == (255, 255, 255, 255)

    def test_stylesheet_and_css_url(self):
        css = b"body{margin:0;background:#00f} .bg{width:20px;height:10px;background:url(img/bg.png)}"
        w, _, rgba = blitz_py.render_rgba(
            "<link rel=stylesheet href='css/site.css'><div class=bg></div>",
            width=40,
            height=20,
            # url() inside the stylesheet resolves against the stylesheet's URL.
            resources={"css/site.css": css, "css/img/bg.png": RED},
        )
        assert pixel(rgba, w, 5, 5) == (255, 0, 0, 255)
        assert pixel(rgba, w, 30, 15) == (0, 0, 255, 255)

    def test_keys_resolve_against_base_url(self):
        resources = {"https://cdn.example/a.png": RED}
        w, _, rgba = blitz_py.render_rgba(
            IMG_DOC % "a.png",
            width=40,
            height=20,
            base_url="https://cdn.example/",
            resources=resources,
        )
        assert pixel(rgba, w, 5, 5) == (255, 0, 0, 255)

    def test_matches_equivalent_data_uri(self):
        # Same bytes as the equivalent data: URI, without base64 in the HTML.
        uri = "data:image/png;base64," + base64.b64encode(RED).decode()
        a = blitz_py.render_png(IMG_DOC % uri, width=40, height=20)
        b = blitz_py.render_png(
            IMG_DOC % "logo.png", width=40, height=20, resources={"logo.png": RED}
        )
        assert a == b

    def test_template_and_layers(self):
        tpl = blitz_py.Template(
            IMG_DOC % "logo.png", width=40, height=20, resources={"logo.png": RED}
        )
        assert tpl.get_box("i")[2:] == (20, 10)
        tpl.set_attribute("i", "src", "logo2.png")  # not provided: missing
        w, _, rgba = blitz_py.render_layers(
            [
                {
                    "html": IMG_DOC % "logo.png",
                    "width": 40,
                    "height": 20,
                    "resources": {"logo.png": RED},
                }
            ],
            width=40,
            height=20,
        )
        assert pixel(rgba, w, 5, 5) == (255, 0, 0, 255)

    def test_animation_entry_points(self):
        _, _, frames = blitz_py.render_frames(
            IMG_DOC % "logo.png", width=40, height=20, times=[0.0], resources={"logo.png": RED}
        )
        assert pixel(frames[0], 40, 5, 5) == (255, 0, 0, 255)
        assert blitz_py.render_gif(
            IMG_DOC % "logo.png", width=40, height=20, times=[0.0], resources={"logo.png": RED}
        )
        assert blitz_py.render_jpeg(
            IMG_DOC % "logo.png", width=40, height=20, resources={"logo.png": RED}
        )

    def test_values_must_be_bytes(self):
        with pytest.raises(TypeError):
            blitz_py.render_png(
                IMG_DOC % "a.css", width=8, height=8, resources={"a.css": "body{}"}
            )
