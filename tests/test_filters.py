"""CSS `filter`, which blitz-paint converts and vello_cpu now paints.

anyrender_vello_cpu's `filters` feature is on. vello_cpu implements blur,
drop shadow, offset and flood; the colour filters (grayscale, sepia, ...)
reach an `unimplemented!()` there, so src/scene.rs drops them instead of
letting the render panic.
"""

import pytest

import blitz_py

W = H = 120


def pixel(css: str, x: int, y: int) -> tuple[int, int, int]:
    """A pixel around a red 40px box at (40, 40) on black, styled with ``css``."""
    html = (
        "<style>body { margin: 0; background: #000 }"
        ".a { position: absolute; left: 40px; top: 40px; width: 40px; height: 40px;"
        f" background: rgb(255, 0, 0); {css} }}</style><div class='a'></div>"
    )
    _, _, rgba = blitz_py.render_rgba(html, width=W, height=H)
    off = (y * W + x) * 4
    return tuple(rgba[off : off + 3])


def test_blur_spreads_past_the_box() -> None:
    assert pixel("", 36, 60) == (0, 0, 0)
    red, _, _ = pixel("filter: blur(4px)", 36, 60)
    assert red > 20
    assert pixel("filter: blur(4px)", 41, 60)[0] < 255  # the edge softens


def test_drop_shadow() -> None:
    assert pixel("filter: drop-shadow(8px 0 0 lime)", 84, 60) == (0, 255, 0)
    _, green, _ = pixel("filter: drop-shadow(0 0 6px lime)", 83, 60)
    assert green > 20
    assert pixel("filter: drop-shadow(8px 0 0 lime)", 60, 60) == (255, 0, 0)


@pytest.mark.parametrize(
    "css",
    [
        "filter: grayscale(1)",
        "filter: sepia(1)",
        "filter: saturate(0)",
        "filter: brightness(0.5)",
        "filter: contrast(2)",
        "filter: invert(1)",
        "filter: hue-rotate(90deg)",
        "filter: opacity(0.5)",
        "filter: grayscale(1) blur(4px)",
        "backdrop-filter: blur(4px)",
    ],
)
def test_unsupported_filters_are_ignored_not_fatal(css: str) -> None:
    """These used to panic inside vello_cpu once filters were enabled."""
    assert pixel(css, 60, 60) == (255, 0, 0)


def test_only_the_first_filter_of_a_chain_applies() -> None:
    """anyrender_vello_cpu converts a filter list's first node only."""
    assert pixel("filter: blur(4px) grayscale(1)", 60, 60) == (255, 0, 0)
    assert pixel("filter: blur(4px) grayscale(1)", 36, 60)[0] > 20


@pytest.mark.xfail(
    strict=True,
    reason="vello_cpu 0.4 displaces a rectangular clip enclosing a filter layer by the "
    "filter's padding, cutting the halo (and the element) on the top/left (docs/UPSTREAM.md)",
)
def test_blur_inside_a_rectangular_overflow_clip() -> None:
    """The blurred box inside an `overflow: hidden` parent 6px wider on each side."""
    html = (
        "<style>body { margin: 0; background: #000 }"
        ".p { position: absolute; left: 34px; top: 34px; width: 52px; height: 52px;"
        " overflow: hidden }"
        ".a { position: absolute; left: 6px; top: 6px; width: 40px; height: 40px;"
        " background: rgb(255, 0, 0); filter: blur(4px) }</style>"
        "<div class='p'><div class='a'></div></div>"
    )
    _, _, rgba = blitz_py.render_rgba(html, width=W, height=H)

    def red(x: int) -> int:
        return rgba[(60 * W + x) * 4]

    assert red(33) == red(86) == 0  # clipped outside the parent
    assert red(60) == 255
    for d in range(1, 6):  # the halo inside the parent, both sides alike
        assert red(40 - d) == red(80 + d - 1) > 0
