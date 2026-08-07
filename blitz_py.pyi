"""Render HTML/CSS to images — no browser required."""

__version__: str

def render_png(
    html: str,
    *,
    width: int,
    height: int | None = None,
    scale: float = 1.0,
    color_scheme: str = "light",
    background: str | None = "#ffffff",
    base_url: str | None = None,
    fonts: list[bytes] | None = None,
    default_font_family: str | None = None,
    allow_file_urls: bool = False,
    css: str | None = None,
    css_vars: dict[str, str] | None = None,
) -> bytes:
    """Render an HTML string to PNG bytes. height=None sizes to content."""

def render_rgba(
    html: str,
    *,
    width: int,
    height: int | None = None,
    scale: float = 1.0,
    color_scheme: str = "light",
    background: str | None = "#ffffff",
    base_url: str | None = None,
    fonts: list[bytes] | None = None,
    default_font_family: str | None = None,
    allow_file_urls: bool = False,
    css: str | None = None,
    css_vars: dict[str, str] | None = None,
) -> tuple[int, int, bytes]:
    """Render to raw RGBA pixels: (width, height, rgba_bytes)."""

def render_frames(
    html: str,
    *,
    width: int,
    height: int,
    times: list[float],
    scale: float = 1.0,
    color_scheme: str = "light",
    background: str | None = "#ffffff",
    base_url: str | None = None,
    fonts: list[bytes] | None = None,
    default_font_family: str | None = None,
    allow_file_urls: bool = False,
    css: str | None = None,
    css_vars: dict[str, str] | None = None,
) -> tuple[int, int, list[bytes]]:
    """Render CSS-animation frames at the given timestamps (seconds)."""

def render_jpeg(
    html: str,
    *,
    width: int,
    height: int | None = None,
    quality: int = 90,
    scale: float = 1.0,
    color_scheme: str = "light",
    background: str | None = "#ffffff",
    base_url: str | None = None,
    fonts: list[bytes] | None = None,
    default_font_family: str | None = None,
    allow_file_urls: bool = False,
    css: str | None = None,
    css_vars: dict[str, str] | None = None,
) -> bytes:
    """Render to JPEG bytes (no alpha — use an opaque background)."""

def render_gif(
    html: str,
    *,
    width: int,
    height: int,
    times: list[float],
    colors: int = 64,
    scale: float = 1.0,
    color_scheme: str = "light",
    background: str | None = "#ffffff",
    base_url: str | None = None,
    fonts: list[bytes] | None = None,
    default_font_family: str | None = None,
    allow_file_urls: bool = False,
    css: str | None = None,
    css_vars: dict[str, str] | None = None,
) -> bytes:
    """Render CSS-animation frames and encode a looping GIF natively."""

def measure_text(
    text: str,
    *,
    font_size: float,
    font_family: str | None = None,
    font_weight: float = 400.0,
    letter_spacing: float = 0.0,
    max_width: float | None = None,
    fonts: list[bytes] | None = None,
) -> tuple[float, float]:
    """Measure text with the rendering engine's own shaper (one source of truth).

    Returns (width, height) in CSS px. font_family accepts a CSS list
    ("Inter, sans-serif"); with max_width the text wraps and height grows.
    """

class Template:
    """A parsed, reusable document for fast repeated renders.

    Mutate elements by their `id` attribute, then re-render (~1ms for
    widget-sized output). Safe to share across threads.
    """

    def __init__(
        self,
        html: str,
        *,
        width: int,
        height: int,
        scale: float = 1.0,
        color_scheme: str = "light",
        background: str | None = "#ffffff",
        base_url: str | None = None,
        fonts: list[bytes] | None = None,
        default_font_family: str | None = None,
        allow_file_urls: bool = False,
        css: str | None = None,
        css_vars: dict[str, str] | None = None,
    ) -> None: ...
    def set_text(self, id: str, text: str) -> None:
        """Replace the text of the element with this id (must contain exactly one text node)."""

    def update(self, **id_to_text: str) -> None:
        """Batch set_text by keyword: update(temp=\"21.5\", hum=\"48%\"). Atomic."""

    def set_html(self, id: str, html: str) -> None:
        """Replace the children of the element with a parsed HTML fragment."""

    def set_style(self, id: str, name: str, value: str) -> None:
        """Set an inline style property on the element with this id."""

    def set_attribute(self, id: str, name: str, value: str) -> None:
        """Set an attribute on the element with this id."""

    def render_png(self, *, time: float = 0.0) -> bytes: ...
    def render_jpeg(self, *, quality: int = 90, time: float = 0.0) -> bytes: ...
    def render_rgba(self, *, time: float = 0.0) -> tuple[int, int, bytes]: ...
    def render_frames(self, *, times: list[float]) -> tuple[int, int, list[bytes]]: ...
    def render_gif(self, *, times: list[float], colors: int = 64) -> bytes: ...
