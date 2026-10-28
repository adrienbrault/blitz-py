"""Hostile-input corpus: rendering must never crash, whatever the input."""

import os
import random
import string

import pytest

import blitz_py


def render(html: str) -> bytes:
    return blitz_py.render_png(html, width=64, height=64)


class TestHostileInputs:
    def test_seeded_garbage(self):
        rng = random.Random(1234)
        alphabet = string.printable + "<>&\"'é中\U0001f600"
        for _ in range(50):
            junk = "".join(rng.choice(alphabet) for _ in range(rng.randint(1, 2000)))
            assert render(junk)

    def test_deep_nesting(self):
        assert render("<div>" * 5000 + "x" + "</div>" * 5000)

    def test_deep_nesting_unclosed(self):
        assert render("<div><span><p>" * 2000)

    def test_huge_attribute(self):
        assert render(f"<div class='{'a' * 500_000}'>x</div>")

    def test_huge_text(self):
        assert render("word " * 200_000)

    def test_null_bytes_and_controls(self):
        assert render("a\x00b\x01c\x02<p>\x7f</p>\x1b[31m")

    def test_broken_entities(self):
        assert render("&#xFFFFFFF; &#0; &notarealentity; &#x110000; &amp")

    def test_script_and_iframe_ignored(self):
        assert render(
            "<script>while(true){}</script><iframe src='http://example.com'>"
            "<object data='x'><embed src='y'>"
        )

    def test_css_calc_bomb(self):
        css = "width: calc(" + "1px + calc(".join([""] * 200) + "1px" + ")" * 200
        assert render(f"<div style='{css}'>x</div>")

    def test_absurd_css_values(self):
        assert render(
            "<div style='width:9e999px;height:-5px;margin:99999999999px;"
            "font-size:1e10px;z-index:99999999999999999999'>x</div>"
        )

    def test_recursive_css_vars(self):
        assert render(
            "<style>:root{--a:var(--b);--b:var(--a)}</style>"
            "<p style='width:var(--a)'>x</p>"
        )

    def test_malformed_data_uris(self):
        assert render(
            "<img src='data:'><img src='data:image/png;base64,!!!'>"
            "<img src='data:;base64'><img src='data:image/png;base64,AAAA'>"
        )

    def test_svg_garbage(self):
        assert render("<svg><path d='M NaN NaN L'/><circle r='-5'/><svg><svg>")

    def test_mixed_direction_text(self):
        assert render("<p>hello ‮evil‬ שלום world</p>")

    def test_extremely_long_word(self):
        assert render("<p>" + "x" * 100_000 + "</p>")

    def test_relative_urls_without_base_url(self):
        # Without base_url, relative URLs used to panic in the engine
        # (pyo3_runtime.PanicException, which `except Exception` misses).
        assert render(
            "<img src='logo.png'><img src='//host/x.png'><img src='/abs.png'>"
            "<link rel=stylesheet href='a.css'><iframe src='frame.html'></iframe>"
            "<style>@font-face{font-family:X;src:url(x.ttf)}</style>"
            "<div style='background:url(bg.png)'>x</div>"
        )
        tpl = blitz_py.Template("<img src='logo.png'>", width=32, height=32)
        assert tpl.render_png()


class TestBaseUrl:
    @pytest.mark.parametrize("bad", ["not a url", "mailto:x@y.z", "data:text/html,x"])
    def test_rejects_unusable_base_url(self, bad):
        with pytest.raises(ValueError, match="base_url"):
            blitz_py.render_png("<img src='a.png'>", width=8, height=8, base_url=bad)

    def test_accepts_hierarchical_base_url(self):
        assert blitz_py.render_png(
            "<img src='a.png'>", width=8, height=8, base_url="https://example.com/x/"
        )


def _gray_png_data_uri(w: int, h: int) -> str:
    """A valid all-black grayscale PNG; huge sizes compress to a few KB."""
    import base64
    import struct
    import zlib

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = zlib.compress(b"\x00" * ((w + 1) * h), 9)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0))
        + chunk(b"IDAT", raw)
        + chunk(b"IEND", b"")
    )
    return "data:image/png;base64," + base64.b64encode(png).decode()


class TestResourceLimits:
    def test_decompression_bomb_image_is_dropped(self):
        # 9000x9000 declares 81M pixels (> the 64M output limit): the engine
        # would decode it in full (~400MB); it must be skipped as missing.
        tpl = blitz_py.Template(
            f"<img id=i src='{_gray_png_data_uri(9000, 9000)}'>", width=64, height=64
        )
        assert tpl.get_box("i")[2] == 0

    def test_normal_image_still_loads(self):
        tpl = blitz_py.Template(
            f"<img id=i src='{_gray_png_data_uri(20, 10)}'>", width=64, height=64
        )
        assert tpl.get_box("i")[2:] == (20, 10)

    @pytest.mark.skipif(not os.path.exists("/dev/zero"), reason="needs /dev/zero")
    def test_endless_file_url_is_bounded(self):
        assert blitz_py.render_png(
            "<img src='file:///dev/zero'><link rel=stylesheet href='file:///dev/zero'>",
            width=8,
            height=8,
            allow_file_urls=True,
        )
