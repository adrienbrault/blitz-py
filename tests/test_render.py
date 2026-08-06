import base64
import struct
import zlib

import pytest

import blitz_py


def make_png(width: int, height: int, rgba: tuple[int, int, int, int]) -> bytes:
    """Build a solid-color RGBA PNG with the stdlib."""

    def chunk(ctype: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + ctype
            + data
            + struct.pack(">I", zlib.crc32(ctype + data))
        )

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    row = b"\x00" + bytes(rgba) * width
    idat = zlib.compress(row * height)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", idat)
        + chunk(b"IEND", b"")
    )


RED_PNG_B64 = base64.b64encode(make_png(4, 4, (255, 0, 0, 255))).decode()


def png_size(png: bytes) -> tuple[int, int]:
    assert png[:8] == b"\x89PNG\r\n\x1a\n"
    w, h = struct.unpack(">II", png[16:24])
    return w, h


def decode_png_rgba(png: bytes) -> tuple[int, int, bytes]:
    """Minimal PNG decoder for our own non-interlaced RGBA8 output."""
    w, h = png_size(png)
    idat = b""
    pos = 8
    while pos < len(png):
        (length,) = struct.unpack(">I", png[pos : pos + 4])
        ctype = png[pos + 4 : pos + 8]
        if ctype == b"IDAT":
            idat += png[pos + 8 : pos + 8 + length]
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = w * 4
    out = bytearray()
    prev = bytearray(stride)
    for y in range(h):
        row_start = y * (stride + 1)
        filter_type = raw[row_start]
        row = bytearray(raw[row_start + 1 : row_start + 1 + stride])
        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(4, stride):
                row[i] = (row[i] + row[i - 4]) & 0xFF
        elif filter_type == 2:
            for i in range(stride):
                row[i] = (row[i] + prev[i]) & 0xFF
        elif filter_type == 3:
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                row[i] = (row[i] + (left + prev[i]) // 2) & 0xFF
        elif filter_type == 4:
            for i in range(stride):
                a = row[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                row[i] = (row[i] + pred) & 0xFF
        out += row
        prev = row
    return w, h, bytes(out)


def pixel(rgba: bytes, w: int, x: int, y: int) -> tuple[int, int, int, int]:
    off = (y * w + x) * 4
    return tuple(rgba[off : off + 4])


class TestBasics:
    def test_dimensions(self):
        png = blitz_py.render_png("<h1>hi</h1>", width=200, height=100)
        assert png_size(png) == (200, 100)

    def test_scale_multiplies_pixels(self):
        png = blitz_py.render_png("<h1>hi</h1>", width=200, height=100, scale=2.0)
        assert png_size(png) == (400, 200)

    def test_rgba_matches_png(self):
        html = "<div style='background:#123456;width:50px;height:50px'></div>"
        w, h, rgba = blitz_py.render_rgba(html, width=60, height=60)
        assert (w, h) == (60, 60)
        assert len(rgba) == 60 * 60 * 4
        pw, ph, prgba = decode_png_rgba(blitz_py.render_png(html, width=60, height=60))
        assert (pw, ph) == (60, 60)
        assert prgba == rgba

    def test_deterministic(self):
        html = "<h1 style='color:rebeccapurple'>Determinism</h1>"
        a = blitz_py.render_png(html, width=300, height=80)
        b = blitz_py.render_png(html, width=300, height=80)
        assert a == b

    def test_version(self):
        assert blitz_py.__version__


class TestPixels:
    def test_background_color(self):
        w, h, rgba = blitz_py.render_rgba("<body>", width=10, height=10, background="#ff0000")
        assert pixel(rgba, w, 5, 5) == (255, 0, 0, 255)

    def test_transparent_background(self):
        w, h, rgba = blitz_py.render_rgba("<body>", width=10, height=10, background=None)
        assert pixel(rgba, w, 5, 5)[3] == 0

    def test_css_paints(self):
        html = "<div style='width:20px;height:20px;background:#00ff00'></div>"
        w, h, rgba = blitz_py.render_rgba(
            "<body style='margin:0'>" + html, width=20, height=20
        )
        assert pixel(rgba, w, 10, 10) == (0, 255, 0, 255)

    def test_text_renders_without_system_font_dependency(self):
        # Bundled Inter guarantees non-background pixels for text everywhere.
        w, h, rgba = blitz_py.render_rgba(
            "<body style='margin:0;background:#fff;color:#000;font-family:sans-serif'>Hello",
            width=200,
            height=50,
        )
        assert any(
            pixel(rgba, w, x, y) != (255, 255, 255, 255)
            for x in range(0, 200, 2)
            for y in range(0, 50, 2)
        )

    def test_data_uri_image(self):
        html = (
            "<body style='margin:0'>"
            f"<img src='data:image/png;base64,{RED_PNG_B64}' "
            "style='width:20px;height:20px'>"
        )
        w, h, rgba = blitz_py.render_rgba(html, width=20, height=20)
        r, g, b, a = pixel(rgba, w, 10, 10)
        assert a == 255 and r > 200 and g < 60 and b < 60


class TestFlexbox:
    def test_centering(self):
        html = (
            "<body style='margin:0'>"
            "<div style='display:flex;align-items:center;justify-content:center;"
            "width:100px;height:100px;background:#fff'>"
            "<div style='width:10px;height:10px;background:#0000ff'></div></div>"
        )
        w, h, rgba = blitz_py.render_rgba(html, width=100, height=100)
        assert pixel(rgba, w, 50, 50) == (0, 0, 255, 255)
        assert pixel(rgba, w, 10, 10) == (255, 255, 255, 255)


class TestColorScheme:
    def test_dark_scheme_media_query(self):
        html = (
            "<style>body { margin:0; background:#fff }"
            "@media (prefers-color-scheme: dark) { body { background:#000 } }</style>"
            "<body>"
        )
        w, h, light = blitz_py.render_rgba(html, width=10, height=10, color_scheme="light")
        _, _, dark = blitz_py.render_rgba(html, width=10, height=10, color_scheme="dark")
        assert pixel(light, w, 5, 5) == (255, 255, 255, 255)
        assert pixel(dark, w, 5, 5) == (0, 0, 0, 255)


class TestErrors:
    def test_zero_dimensions(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=0, height=100)

    def test_negative_scale(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=10, height=10, scale=-1.0)

    def test_huge_dimensions(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=100_000, height=100_000)

    def test_bad_color_scheme(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=10, height=10, color_scheme="sepia")

    def test_bad_background(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=10, height=10, background="notacolor")

    def test_unknown_default_font_family(self):
        with pytest.raises(ValueError):
            blitz_py.render_png(
                "<p>x</p>", width=10, height=10, default_font_family="No Such Font 123"
            )

    def test_garbage_html_does_not_crash(self):
        png = blitz_py.render_png("<<<>>>\x00&&&<p", width=50, height=50)
        assert png_size(png) == (50, 50)

    def test_garbage_font_bytes_do_not_crash(self):
        png = blitz_py.render_png(
            "<p>x</p>", width=50, height=50, fonts=[b"not a font at all"]
        )
        assert png_size(png) == (50, 50)
