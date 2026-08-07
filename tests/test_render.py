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


class TestFontFace:
    def test_font_face_data_uri(self):
        # Requires the whole @font-face pipeline (data URI fetch -> format
        # sniff -> register under the CSS family name) not to drop the source.
        import pathlib

        font_path = pathlib.Path(__file__).parent.parent / "assets" / "InterVariable.ttf"
        b64 = base64.b64encode(font_path.read_bytes()).decode()
        html = (
            "<style>"
            f"@font-face {{ font-family: TestFace; src: url(data:font/ttf;base64,{b64}) format(truetype); }}"
            "body { margin:0; background:#fff; color:#000 }"
            "h1 { font-family: TestFace; font-size: 40px }"
            "</style><body><h1>Ink</h1>"
        )
        w, h, rgba = blitz_py.render_rgba(html, width=200, height=80)
        assert any(
            pixel(rgba, w, x, y) != (255, 255, 255, 255)
            for x in range(0, 200, 2)
            for y in range(0, 80, 2)
        )


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


class TestAutoHeight:
    def test_height_follows_content(self):
        short = blitz_py.render_png("<p style='margin:0'>one line</p>", width=300)
        tall = blitz_py.render_png(
            "<p style='margin:0'>line</p>" * 30, width=300
        )
        assert png_size(short)[0] == 300 and png_size(tall)[0] == 300
        assert png_size(tall)[1] > png_size(short)[1] >= 10

    def test_scale_applies(self):
        a = blitz_py.render_png("<p style='margin:0;height:100px'>x</p>", width=100)
        b = blitz_py.render_png(
            "<p style='margin:0;height:100px'>x</p>", width=100, scale=2.0
        )
        assert png_size(b)[1] == 2 * png_size(a)[1]


class TestCssInjection:
    def test_css_overrides(self):
        w, h, rgba = blitz_py.render_rgba(
            "<body style='background:#fff'>",
            width=10,
            height=10,
            css="body { background: #00ff00 !important }",
        )
        assert pixel(rgba, w, 5, 5) == (0, 255, 0, 255)

    def test_css_vars(self):
        html = "<body style='margin:0'><div style='width:20px;height:20px;background:var(--accent)'></div>"
        w, h, rgba = blitz_py.render_rgba(
            html, width=20, height=20, css_vars={"accent": "#ff0000"}
        )
        assert pixel(rgba, w, 10, 10) == (255, 0, 0, 255)

    def test_bad_var_name(self):
        with pytest.raises(ValueError):
            blitz_py.render_png(
                "<p>x</p>", width=10, height=10, css_vars={"bad name": "red"}
            )

    def test_var_value_cannot_break_out_of_style(self):
        with pytest.raises(ValueError):
            blitz_py.render_png(
                "<p>x</p>",
                width=10,
                height=10,
                css_vars={"v": "red</style><script>"},
            )


class TestTemplate:
    HTML = (
        "<style>body{margin:0;background:#fff;font-family:sans-serif}</style>"
        "<body><div id='box' style='width:30px;height:30px;background:#00f'></div>"
        "<p id='label'>initial</p></body>"
    )

    def test_set_text_changes_output(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=100)
        a = tpl.render_png()
        tpl.set_text("label", "changed text here")
        b = tpl.render_png()
        assert a != b
        assert png_size(a) == png_size(b) == (200, 100)

    def test_set_style(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=100)
        w, h, before = tpl.render_rgba()
        assert pixel(before, w, 100, 15) == (255, 255, 255, 255)
        tpl.set_style("box", "width", "150px")
        w, h, after = tpl.render_rgba()
        assert pixel(after, w, 100, 15) == (0, 0, 255, 255)

    def test_unknown_id(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=50)
        with pytest.raises(ValueError):
            tpl.set_text("nope", "x")

    def test_set_style_rejects_bad_property_name(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=50)
        with pytest.raises(ValueError):
            tpl.set_style("box", "width:1;height", "10px")

    def test_set_attribute_rejects_id(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=50)
        with pytest.raises(ValueError):
            tpl.set_attribute("box", "id", "renamed")

    def test_matches_one_shot_render(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=100)
        assert tpl.render_png() == blitz_py.render_png(
            self.HTML, width=200, height=100
        )

    def test_cross_thread_use(self):
        from concurrent.futures import ThreadPoolExecutor

        tpl = blitz_py.Template(self.HTML, width=100, height=50)
        with ThreadPoolExecutor(4) as pool:
            results = list(pool.map(lambda i: tpl.render_png(), range(16)))
        assert all(r == results[0] for r in results)


class TestJpeg:
    def test_magic_and_size(self):
        j = blitz_py.render_jpeg("<h1>hi</h1>", width=200, height=100)
        assert j[:3] == b"\xff\xd8\xff"
        lo = blitz_py.render_jpeg("<h1>hi</h1>", width=200, height=100, quality=10)
        hi = blitz_py.render_jpeg("<h1>hi</h1>", width=200, height=100, quality=95)
        assert len(lo) < len(hi)

    def test_auto_height(self):
        j = blitz_py.render_jpeg("<p style='margin:0;height:50px'>x</p>", width=100)
        assert j[:3] == b"\xff\xd8\xff"

    def test_bad_quality(self):
        with pytest.raises(ValueError):
            blitz_py.render_jpeg("<p>x</p>", width=10, height=10, quality=0)

    def test_template_jpeg(self):
        tpl = blitz_py.Template("<p id='x'>hello</p>", width=100, height=40)
        assert tpl.render_jpeg()[:3] == b"\xff\xd8\xff"


GIF_ANIM = (
    "<style>body{margin:0;background:#fff}"
    ".b{width:20px;height:20px;background:#e11;animation:m 1s linear infinite}"
    "@keyframes m{from{transform:translateX(0)}to{transform:translateX(60px)}}"
    "</style><body><div class='b'></div></body>"
)


class TestGif:
    def test_gif_structure(self):
        g = blitz_py.render_gif(
            GIF_ANIM, width=100, height=30, times=[i / 10 for i in range(10)]
        )
        assert g[:6] == b"GIF89a"
        assert b"NETSCAPE2.0" in g[:1024]  # infinite loop extension

    def test_gif_reasonable_size(self):
        g = blitz_py.render_gif(
            GIF_ANIM, width=100, height=30, times=[i / 10 for i in range(10)]
        )
        assert len(g) < 8_000

    def test_gif_decodes_and_animates(self):
        import io

        PIL = pytest.importorskip("PIL.Image")
        g = blitz_py.render_gif(
            GIF_ANIM, width=100, height=30, times=[0.0, 0.5]
        )
        im = PIL.open(io.BytesIO(g))
        assert im.n_frames == 2
        f0 = im.convert("RGB").getpixel((5, 10))
        im.seek(1)
        f1 = im.convert("RGB").getpixel((5, 10))
        assert f0 != f1  # box moved away from (5,10)

    def test_colors_validation(self):
        with pytest.raises(ValueError):
            blitz_py.render_gif("<p>x</p>", width=10, height=10, times=[0.0], colors=1)

    def test_template_gif(self):
        tpl = blitz_py.Template(GIF_ANIM, width=100, height=30)
        g = tpl.render_gif(times=[0.0, 0.25, 0.5])
        assert g[:6] == b"GIF89a"


class TestMeasureText:
    def test_monotone_in_length(self):
        w1, h1 = blitz_py.measure_text("hello", font_size=16)
        w2, h2 = blitz_py.measure_text("hello world, longer", font_size=16)
        assert 0 < w1 < w2
        assert h1 == h2 > 0

    def test_scales_with_font_size(self):
        w1, _ = blitz_py.measure_text("hello", font_size=16)
        w2, _ = blitz_py.measure_text("hello", font_size=32)
        assert w2 == pytest.approx(w1 * 2, rel=0.01)

    def test_weight_widens(self):
        w1, _ = blitz_py.measure_text("hello", font_size=16, font_weight=400)
        w2, _ = blitz_py.measure_text("hello", font_size=16, font_weight=700)
        assert w2 > w1

    def test_wrapping_height(self):
        text = "many words " * 20
        _, h1 = blitz_py.measure_text(text, font_size=16)
        _, h2 = blitz_py.measure_text(text, font_size=16, max_width=120.0)
        assert h2 > h1 * 3

    def test_agrees_with_rendering(self):
        # A box exactly as wide as the measured text must fit it on one line;
        # one 20% narrower must wrap to two. Verified via auto-height.
        text = "The quick brown fox"
        w, line_h = blitz_py.measure_text(text, font_size=16)
        html = (
            "<style>body{margin:0;font-family:sans-serif;font-size:16px}</style>"
            f"<body><div style='width:{w + 1:.0f}px'>{text}</div>"
        )
        png_fit = blitz_py.render_png(html, width=400)
        html2 = html.replace(f"width:{w + 1:.0f}px", f"width:{w * 0.8:.0f}px")
        png_wrap = blitz_py.render_png(html2, width=400)
        assert png_size(png_wrap)[1] > png_size(png_fit)[1]

    def test_validation(self):
        with pytest.raises(ValueError):
            blitz_py.measure_text("x", font_size=0)
        with pytest.raises(ValueError):
            blitz_py.measure_text("x", font_size=16, font_weight=0)


class TestTemplateV3:
    HTML = (
        "<style>body{margin:0;background:#fff;font-family:sans-serif}</style>"
        "<body><div id='list'><p id='row'>old row</p></div>"
        "<p id='a'>aaa</p><p id='b'>bbb</p></body>"
    )

    def test_set_html_replaces_region(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=120)
        before = tpl.render_png()
        tpl.set_html(
            "list",
            "<div style='width:40px;height:12px;background:#00f'></div>" * 3,
        )
        after = tpl.render_png()
        assert before != after

    def test_set_html_new_ids_usable(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=120)
        tpl.set_html("list", "<p id='fresh'>inserted</p>")
        a = tpl.render_png()
        tpl.set_text("fresh", "changed")
        assert tpl.render_png() != a
        # old id inside the replaced region is gone
        with pytest.raises(ValueError):
            tpl.set_text("row", "x")

    def test_update_batch(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=120)
        before = tpl.render_png()
        tpl.update(a="AAA!", b="BBB!")
        assert tpl.render_png() != before

    def test_update_is_atomic(self):
        tpl = blitz_py.Template(self.HTML, width=200, height=120)
        before = tpl.render_png()
        with pytest.raises(ValueError):
            tpl.update(a="new", nosuchid="x")
        assert tpl.render_png() == before

    def test_render_frames_with_mutation(self):
        tpl = blitz_py.Template(GIF_ANIM, width=100, height=30)
        w, h, frames = tpl.render_frames(times=[0.0, 0.5])
        assert (w, h) == (100, 30)
        assert frames[0] != frames[1]


ANIMATED_HTML = (
    "<style>body{margin:0;background:#fff}"
    ".b{width:20px;height:20px;background:#e11;animation:m 2s linear infinite}"
    "@keyframes m{from{transform:translateX(0)}to{transform:translateX(80px)}}"
    "</style><body><div class='b'></div></body>"
)


class TestFrames:
    def test_animation_advances(self):
        w, h, frames = blitz_py.render_frames(
            ANIMATED_HTML, width=100, height=40, times=[0.0, 0.5, 1.0]
        )
        assert (w, h) == (100, 40)
        assert len(frames) == 3
        assert frames[0] != frames[1] != frames[2]
        # box starts at x=0: red at (5,10) in frame 0, white in frame 2
        assert pixel(frames[0], w, 5, 10)[0] > 200
        assert pixel(frames[2], w, 5, 10) == (255, 255, 255, 255)
        # at t=1.0 (50%) the box straddles x=40..60
        assert pixel(frames[2], w, 50, 10)[0] > 200

    def test_deterministic_frames(self):
        _, _, a = blitz_py.render_frames(ANIMATED_HTML, width=100, height=40, times=[0.75])
        _, _, b = blitz_py.render_frames(ANIMATED_HTML, width=100, height=40, times=[0.75])
        assert a == b

    def test_static_content_stable(self):
        _, _, frames = blitz_py.render_frames(
            "<body style='background:#123'>", width=20, height=20, times=[0.0, 5.0]
        )
        assert frames[0] == frames[1]

    def test_times_validation(self):
        with pytest.raises(ValueError):
            blitz_py.render_frames("<p>x</p>", width=10, height=10, times=[])
        with pytest.raises(ValueError):
            blitz_py.render_frames("<p>x</p>", width=10, height=10, times=[-1.0])
        with pytest.raises(ValueError):
            blitz_py.render_frames(
                "<p>x</p>", width=10, height=10, times=[0.0] * 1001
            )


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

    def test_non_ascii_background(self):
        with pytest.raises(ValueError):
            blitz_py.render_png("<p>x</p>", width=10, height=10, background="#€€€")

    def test_unknown_default_font_family(self):
        with pytest.raises(ValueError):
            blitz_py.render_png(
                "<p>x</p>", width=10, height=10, default_font_family="No Such Font 123"
            )

    def test_garbage_html_does_not_crash(self):
        png = blitz_py.render_png("<<<>>>\x00&&&<p", width=50, height=50)
        assert png_size(png) == (50, 50)

    def test_bundled_family_always_available(self):
        # The bundled font is registered under the explicit name "Inter"
        # regardless of what fonts the system has.
        png = blitz_py.render_png(
            "<p>x</p>", width=10, height=10, default_font_family="Inter"
        )
        assert png_size(png) == (10, 10)

    def test_fonts_do_not_leak_between_renders(self):
        # A font registered in one render must not make its family visible to
        # later renders (the shared base collection must stay pristine).
        # InterVariable.ttf's own family name is "Inter Variable", which only
        # exists in a render that registered it via fonts=.
        import pathlib

        font = (
            pathlib.Path(__file__).parent.parent / "assets" / "InterVariable.ttf"
        ).read_bytes()
        blitz_py.render_png(
            "<p>x</p>",
            width=10,
            height=10,
            fonts=[font],
            default_font_family="Inter Variable",
        )
        with pytest.raises(ValueError):
            blitz_py.render_png(
                "<p>x</p>", width=10, height=10, default_font_family="Inter Variable"
            )

    def test_garbage_font_bytes_do_not_crash(self):
        png = blitz_py.render_png(
            "<p>x</p>", width=50, height=50, fonts=[b"not a font at all"]
        )
        assert png_size(png) == (50, 50)
