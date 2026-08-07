"""Hostile-input corpus: rendering must never crash, whatever the input."""

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
