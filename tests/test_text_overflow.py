"""`text-overflow: ellipsis`, painted by blitz-py's Blitz (adrienbrault/blitz#2).

Before, the engine clipped overflowing text without a marker. Now a box
that clips its inline overflow (`overflow: hidden`, `white-space: nowrap`)
ends an overflowing line with "…"; text that fits is drawn as it was.
"""

import blitz_py

W, H = 120, 30


def render(text: str, overflow: str) -> bytes:
    html = (
        "<style>body { margin: 0; background: #fff; color: #000;"
        " font: 16px/30px sans-serif }"
        f".t {{ width: {W}px; white-space: nowrap; overflow: hidden; text-overflow: {overflow} }}"
        f"</style><div class='t'>{text}</div>"
    )
    return blitz_py.render_rgba(html, width=W, height=H)[2]


def ink_columns(rgba: bytes) -> list[int]:
    """Columns holding any dark pixel."""
    return [x for x in range(W) if any(rgba[(y * W + x) * 4] < 128 for y in range(H))]


LONG = "Living room temperature sensor"


def test_overflowing_text_ends_with_an_ellipsis() -> None:
    clipped = render(LONG, "clip")
    ellipsized = render(LONG, "ellipsis")
    assert clipped != ellipsized
    # The marker replaces the cut glyphs: the clip runs ink to the edge, the
    # ellipsis ends on three dots with a gap before the box edge.
    assert ink_columns(clipped)[-1] >= W - 2
    marker = render("…", "clip")
    width = ink_columns(marker)[-1] - ink_columns(marker)[0]
    end = ink_columns(ellipsized)[-1]
    assert end < W - 1
    # The last ink is the ellipsis: dots on the baseline, nothing above them.
    tail = range(end - width, end + 1)
    rows = {y for x in tail for y in range(H) if ellipsized[(y * W + x) * 4] < 128}
    assert max(rows) - min(rows) <= 3, sorted(rows)


def test_text_that_fits_is_unchanged() -> None:
    assert render("21.5°", "ellipsis") == render("21.5°", "clip")


def test_ellipsis_needs_a_clipping_box() -> None:
    """Without `overflow: hidden` the line overflows visibly, unmarked."""
    html = (
        "<style>body { margin: 0; background: #fff; font: 16px/30px sans-serif }"
        f".t {{ width: {W}px; white-space: nowrap; text-overflow: OVERFLOW }}</style>"
        f"<div class='t'>{LONG}</div>"
    )
    a = blitz_py.render_rgba(html.replace("OVERFLOW", "clip"), width=W, height=H)[2]
    b = blitz_py.render_rgba(html.replace("OVERFLOW", "ellipsis"), width=W, height=H)[2]
    assert a == b
