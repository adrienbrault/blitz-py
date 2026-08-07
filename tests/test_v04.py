"""v0.4.0: text utilities, register_fonts, box introspection, layers."""

import pathlib

import pytest

import blitz_py

STYLE = dict(font_size=16)
INTER = (pathlib.Path(__file__).parent.parent / "assets" / "InterVariable.ttf").read_bytes()


def pixel(rgba: bytes, w: int, x: int, y: int) -> tuple[int, int, int, int]:
    off = (y * w + x) * 4
    return tuple(rgba[off : off + 4])


class TestTextUtils:
    def test_ellipsize_noop_when_fits(self):
        assert blitz_py.ellipsize("short", max_width=500.0, **STYLE) == "short"

    def test_ellipsize_fits_and_marks(self):
        text = "a very long caption that cannot possibly fit in the box"
        out = blitz_py.ellipsize(text, max_width=120.0, **STYLE)
        assert out.endswith("…") and len(out) < len(text)
        w, _ = blitz_py.measure_text(out, **STYLE)
        assert w <= 120.0

    def test_line_clamp(self):
        text = "many words that will wrap over quite a few lines " * 4
        out = blitz_py.line_clamp(text, max_width=150.0, max_lines=2, **STYLE)
        assert out.endswith("…")
        lines = blitz_py.measure_text_lines(out, max_width=150.0, **STYLE)
        assert len(lines) <= 2

    def test_fit_font_size(self):
        size = blitz_py.fit_font_size("23.5°C", max_width=100.0, max_size=80.0)
        assert 6.0 <= size <= 80.0
        w, _ = blitz_py.measure_text("23.5°C", font_size=size)
        assert w <= 100.0
        w_bigger, _ = blitz_py.measure_text("23.5°C", font_size=size + 1.0)
        assert w_bigger > 100.0

    def test_fit_font_size_cap(self):
        assert blitz_py.fit_font_size("hi", max_width=10_000.0, max_size=40.0) == 40.0

    def test_wrap_balanced(self):
        text = "the quick brown fox jumps over the lazy dog again and again"
        lines = blitz_py.wrap_balanced(text, max_width=180.0, **STYLE)
        assert " ".join(lines) == text
        for line in lines:
            w, _ = blitz_py.measure_text(line, **STYLE)
            assert w <= 180.0
        # balanced: last line is not a lonely orphan word when multiple lines
        if len(lines) > 1:
            assert len(lines[-1].split()) >= 1

    def test_measure_text_lines(self):
        one = blitz_py.measure_text_lines("hello", **STYLE)
        assert len(one) == 1 and one[0][0] > 0
        many = blitz_py.measure_text_lines("word " * 30, max_width=100.0, **STYLE)
        assert len(many) > 3


class TestRegisterFonts:
    def test_register_returns_names_and_enables_family(self):
        names = blitz_py.register_fonts([INTER])
        assert "Inter Variable" in names
        # family usable without per-call fonts=
        w, _ = blitz_py.measure_text("abc", font_size=16, font_family="Inter Variable")
        assert w > 0


class TestBoxes:
    HTML = (
        "<style>body{margin:0}</style>"
        "<body><div id='pad' style='padding:10px 0 0 20px'>"
        "<div id='box' style='width:30px;height:40px'></div></div></body>"
    )

    def test_get_box(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=100)
        x, y, w, h = tpl.get_box("box")
        assert (x, y, w, h) == (20.0, 10.0, 30.0, 40.0)

    def test_boxes(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=100)
        boxes = tpl.boxes()
        assert set(boxes) == {"pad", "box"}
        assert boxes["box"][2:] == (30.0, 40.0)

    def test_box_after_mutation(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=100)
        tpl.set_style("box", "height", "77px")
        assert tpl.get_box("box")[3] == 77.0

    def test_unknown_id(self):
        tpl = blitz_py.Template(self.HTML, width=100, height=100)
        with pytest.raises(ValueError):
            tpl.get_box("nope")


RED = "<body style='margin:0'><div style='width:20px;height:20px;background:#f00'></div>"


class TestLayers:
    def test_two_layers_positioning(self):
        w, h, rgba = blitz_py.render_layers(
            [
                {"html": RED, "width": 20, "height": 20, "x": 0, "y": 0},
                {"html": RED.replace("#f00", "#0f0"), "width": 20, "height": 20, "x": 30, "y": 30},
            ],
            width=60,
            height=60,
            background="#000000",
        )
        assert (w, h) == (60, 60)
        assert pixel(rgba, w, 5, 5)[:3] == (255, 0, 0)
        assert pixel(rgba, w, 35, 35)[:3] == (0, 255, 0)
        assert pixel(rgba, w, 55, 5)[:3] == (0, 0, 0)

    def test_template_layer(self):
        tpl = blitz_py.Template(RED, width=20, height=20, background=None)
        w, h, rgba = blitz_py.render_layers(
            [{"template": tpl, "x": 10, "y": 10}], width=40, height=40
        )
        assert pixel(rgba, w, 15, 15)[:3] == (255, 0, 0)
        assert pixel(rgba, w, 5, 5)[:3] == (0, 0, 0)

    def test_paint_order(self):
        w, h, rgba = blitz_py.render_layers(
            [
                {"html": RED, "width": 20, "height": 20},
                {"html": RED.replace("#f00", "#00f"), "width": 20, "height": 20},
            ],
            width=20,
            height=20,
        )
        assert pixel(rgba, w, 10, 10)[:3] == (0, 0, 255)

    def test_opacity(self):
        w, h, rgba = blitz_py.render_layers(
            [{"html": RED, "width": 20, "height": 20, "opacity": 0.5}],
            width=20,
            height=20,
            background="#000000",
        )
        r = pixel(rgba, w, 10, 10)[0]
        assert 100 < r < 155

    def test_blur_spreads(self):
        w, h, rgba = blitz_py.render_layers(
            [{"html": RED, "width": 40, "height": 40, "blur": 6.0}],
            width=40,
            height=40,
            background="#000000",
        )
        # pixels just outside the 20x20 square now carry red glow
        assert pixel(rgba, w, 24, 10)[0] > 10

    def test_tint(self):
        w, h, rgba = blitz_py.render_layers(
            [{"html": RED, "width": 20, "height": 20, "tint": "#00ff00"}],
            width=20,
            height=20,
        )
        assert pixel(rgba, w, 10, 10)[:3] == (0, 255, 0)

    def test_encoded_variants(self):
        layers = [{"html": RED, "width": 20, "height": 20}]
        png = blitz_py.render_layers_png(layers, width=20, height=20)
        assert png[:8] == b"\x89PNG\r\n\x1a\n"
        jpg = blitz_py.render_layers_jpeg(layers, width=20, height=20, quality=85)
        assert jpg[:3] == b"\xff\xd8\xff"

    def test_layer_validation(self):
        with pytest.raises(ValueError):
            blitz_py.render_layers([{}], width=10, height=10)
        with pytest.raises(ValueError):
            blitz_py.render_layers(
                [{"html": RED, "width": 10, "height": 10, "opacity": 2.0}],
                width=10,
                height=10,
            )
        tpl = blitz_py.Template(RED, width=10, height=10)
        with pytest.raises(ValueError):
            blitz_py.render_layers(
                [{"template": tpl, "html": RED}], width=10, height=10
            )
