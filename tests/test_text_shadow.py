"""`text-shadow`, which the engine paints since Blitz #984.

An unblurred shadow is the glyphs drawn again, offset, in the shadow
colour. A blurred one is drawn in a `blur()` filter layer, so it needs
anyrender_vello_cpu's `filters` feature, and src/scene.rs places the
filter's region clip (vello_cpu displaced it, cutting the halo above and
left of the text). Shadows paint beneath the text, the first one on top.
"""

import blitz_py

W, H = 160, 80
ROW = 40  # through the middle of the stem


def row(css: str) -> list[tuple[int, int, int]]:
    """Middle row of a white "I" on black, its text-shadow ``css``.

    The bundled font (sans-serif resolves to it on every platform) draws "I"
    as a plain stem, so shadows have straight, symmetric edges.
    """
    html = (
        "<style>body { margin: 0; background: #000 }"
        ".t { position: absolute; left: 70px; top: 0; color: #fff;"
        " font: 900 60px/80px sans-serif;"
        f" text-shadow: {css} }}</style><div class='t'>I</div>"
    )
    _, _, rgba = blitz_py.render_rgba(html, width=W, height=H)
    return [tuple(rgba[(ROW * W + x) * 4 : (ROW * W + x) * 4 + 3]) for x in range(W)]


def stem() -> tuple[int, int, int]:
    """First column, last column and middle of the stem."""
    xs = [x for x, (r, g, b) in enumerate(row("none")) if min(r, g, b) > 128]
    return xs[0], xs[-1], (xs[0] + xs[-1]) // 2


def gap() -> int:
    """An offset that puts the shadow clear of the stem, 6px apart."""
    left, right, _ = stem()
    return right - left + 7


def test_offset_shadow_draws_the_glyphs_again() -> None:
    left, right, mid = stem()
    r = row(f"{gap()}px 0 0 red")
    assert r[mid + gap()] == (255, 0, 0)
    assert r[right + 3] == (0, 0, 0)
    assert r[left - 3] == (0, 0, 0)
    assert r[mid] == (255, 255, 255)  # beneath the text


def test_negative_offset() -> None:
    _, right, mid = stem()
    r = row(f"-{gap()}px 0 0 red")
    assert r[mid - gap()] == (255, 0, 0)
    assert r[right + 3] == (0, 0, 0)


def test_blurred_shadow_halo_on_both_sides() -> None:
    left, right, _ = stem()
    r = row("0 0 8px red")  # blur radius 8px: std dev 4px
    for d in (1, 2, 4):
        assert r[left - d][0] > 10, (d, r[left - 12 : left + 1])
        assert abs(r[left - d][0] - r[right + d][0]) <= 4, (d, r[left - d], r[right + d])
    assert r[left - 16] == r[right + 16] == (0, 0, 0)


def test_multiple_shadows_first_on_top() -> None:
    _, right, mid = stem()
    r = row(f"{gap()}px 0 0 red, {2 * gap()}px 0 0 lime")
    assert r[mid + gap()] == (255, 0, 0)
    assert r[mid + 2 * gap()] == (0, 255, 0)
    assert r[right + 3] == (0, 0, 0)


def test_blurred_and_offset_shadows_combine() -> None:
    left, _, mid = stem()
    r = row(f"0 0 8px red, {gap()}px 0 0 lime")
    assert r[left - 2][0] > 10  # the blur's halo
    assert r[mid + gap()] == (0, 255, 0)
