"""`text-shadow`, which the engine paints since Blitz #984.

An unblurred shadow is the glyphs drawn again, offset, in the shadow
colour. A blurred one is drawn in a `blur()` filter layer, so it needs
anyrender_vello_cpu's `filters` feature, and src/scene.rs places the
filter's region clip (vello_cpu displaced it, cutting the halo above and
left of the text). Shadows paint beneath the text, the first one on top.
"""

import blitz_py

W, H = 160, 80
ROW = 40  # through the middle of the bar


def row(css: str) -> list[tuple[int, int, int]]:
    """Middle row of a white 20x40 bar at x 60..80 on black, its text-shadow ``css``.

    The bar is a block of `line-height: 40px` text whose only glyph is the
    bundled font's full block (U+2588), so shadows have straight edges.
    """
    html = (
        "<style>body { margin: 0; background: #000 }"
        ".t { position: absolute; left: 60px; top: 20px; width: 20px; height: 40px;"
        " overflow: visible; color: #fff; font: 40px/40px monospace;"
        f" text-shadow: {css} }}</style><div class='t'>&#x2588;</div>"
    )
    _, _, rgba = blitz_py.render_rgba(html, width=W, height=H)
    return [tuple(rgba[(ROW * W + x) * 4 : (ROW * W + x) * 4 + 3]) for x in range(W)]


def edges(css: str = "none") -> tuple[int, int]:
    """First and last column of the bar."""
    xs = [x for x, (r, g, b) in enumerate(row(css)) if min(r, g, b) > 128]
    return xs[0], xs[-1]


def test_offset_shadow_draws_the_glyphs_again() -> None:
    left, right = edges()
    r = row("8px 0 0 red")
    assert r[right + 1] == r[right + 8] == (255, 0, 0)
    assert r[right + 10] == (0, 0, 0)
    assert r[left - 2] == (0, 0, 0)
    assert r[(left + right) // 2] == (255, 255, 255)  # beneath the text


def test_negative_offset() -> None:
    left, right = edges()
    r = row("-8px 0 0 red")
    assert r[left - 1] == r[left - 8] == (255, 0, 0)
    assert r[right + 2] == (0, 0, 0)


def test_blurred_shadow_halo_on_both_sides() -> None:
    left, right = edges()
    r = row("0 0 8px red")  # blur radius 8px: std dev 4px
    for d in (1, 2, 4):
        assert r[left - d][0] > 10, (d, r[left - 12 : left + 1])
        assert abs(r[left - d][0] - r[right + d][0]) <= 2, (d, r[left - d], r[right + d])
    assert r[left - 16] == r[right + 16] == (0, 0, 0)


def test_multiple_shadows_first_on_top() -> None:
    _, right = edges()
    r = row("8px 0 0 red, 16px 0 0 lime")
    assert r[right + 4] == (255, 0, 0)
    assert r[right + 12] == (0, 255, 0)
    assert r[right + 20] == (0, 0, 0)


def test_blurred_and_offset_shadows_combine() -> None:
    left, right = edges()
    r = row("0 0 4px red, 12px 0 0 lime")
    assert r[left - 3][0] > 10  # the blur's halo
    assert r[right + 8] == (0, 255, 0)
