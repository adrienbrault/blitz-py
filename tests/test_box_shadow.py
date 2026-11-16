"""Box-shadow colours outside the sRGB gamut (oklch, display-p3, Tailwind v4).

blitz-paint converts the colour to sRGB without gamut mapping, and
vello_cpu's blurred-rounded-rect fill (every blurred or spread shadow)
converts the raw channel to u8 with SIMD: a channel below 0 becomes 255 on
AVX-512, wraps on AVX2 and saturates to 0 on NEON, and a channel above 1
wraps dark everywhere. An emerald glow came out red (x86) and output
differed by CPU. Fills clamp; shadows now do too (src/scene.rs,
docs/UPSTREAM.md). The below-0 cases only fail without the fix on x86;
the above-1 case fails on every platform.
"""

import pytest

import blitz_py

W = H = 120


def pixel(rgba: bytes, x: int, y: int) -> tuple[int, int, int, int]:
    off = (y * W + x) * 4
    return tuple(rgba[off : off + 4])


def shadow_pixel(shadow: str) -> tuple[int, int, int, int]:
    """A pixel in the shadow just left of a dark 40px box on black."""
    html = (
        "<style>body { margin: 0; background: #000 }"
        ".a { position: absolute; left: 40px; top: 40px; width: 40px; height: 40px;"
        f" background: #333; box-shadow: {shadow} }}</style><div class='a'></div>"
    )
    _, _, rgba = blitz_py.render_rgba(html, width=W, height=H)
    return pixel(rgba, 34, 60)


@pytest.mark.parametrize(
    "color",
    [
        "oklch(76.5% 0.177 163.223)",  # Tailwind v4 emerald-400: red below 0 in sRGB
        "color(srgb -0.2 0.8 0.5)",
        "color(display-p3 0 0.85 0.55)",
    ],
)
@pytest.mark.parametrize("geometry", ["0 0 16px", "0 0 0 8px"])  # blurred, spread
def test_out_of_gamut_shadow_keeps_its_hue(color: str, geometry: str) -> None:
    r, g, _, _ = shadow_pixel(f"{geometry} {color}")
    assert g > 30, (r, g)
    assert r < g / 4, f"{color} {geometry} painted red: {(r, g)}"


def test_shadow_matches_the_clamped_colour() -> None:
    """Clamping per channel is what a fill of the same colour paints."""
    assert shadow_pixel("0 0 0 8px color(srgb -0.2 0.8 0.5)") == shadow_pixel(
        "0 0 0 8px color(srgb 0 0.8 0.5)"
    )


def test_in_gamut_shadows_are_unchanged() -> None:
    assert shadow_pixel("0 0 0 8px rgb(0, 204, 128)")[:3] == (0, 204, 128)


def test_channel_above_one_does_not_wrap_dark() -> None:
    """Above 1 wraps on every SIMD level (below 0 only on some): clamped now."""
    _, g, _, _ = shadow_pixel("0 0 0 8px color(srgb 0.2 1.3 0.5)")
    assert g == 255
