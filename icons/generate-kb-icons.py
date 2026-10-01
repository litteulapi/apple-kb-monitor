#!/usr/bin/env python3
"""Generate the symbolic keyboard-battery icon set of the tray (issue #116).

Output: icons/hicolor/scalable/status/apihub-kb-*-symbolic.svg

* 16x16 symbolic SVGs, **fill only** (no strokes): GTK recolours `-symbolic`
  files by overriding `fill`, so outlines are drawn as even-odd paths.
* `fill="currentColor"` + `class="ColorScheme-Text"` and the Breeze
  `current-color-scheme` stylesheet: Plasma swaps the stylesheet for the
  current colour scheme (light and dark from the same file).
* State classes: the caution gauge carries `ColorScheme-NegativeText` (Plasma)
  and `error` (GTK); the charging bolt carries `ColorScheme-PositiveText` and
  `success`.

Set: 11 levels (000..100) x {discharging, charging}, caution, disconnected,
missing.  Run from anywhere: `python3 icons/generate-kb-icons.py`.
"""

from pathlib import Path

OUT = Path(__file__).resolve().parent / "hicolor" / "scalable" / "status"

STYLE = (
    '<style id="current-color-scheme" type="text/css">'
    ".ColorScheme-Text{color:#232629;}"
    ".ColorScheme-NegativeText{color:#da4453;}"
    ".ColorScheme-PositiveText{color:#27ae60;}"
    "</style>"
)


def rrect(x, y, w, h, r):
    """Closed rounded-rectangle subpath."""
    return (
        f"M{x + r},{y}h{w - 2 * r}a{r},{r} 0 0 1 {r},{r}v{h - 2 * r}"
        f"a{r},{r} 0 0 1 {-r},{r}h{-(w - 2 * r)}a{r},{r} 0 0 1 {-r},{-r}"
        f"v{-(h - 2 * r)}a{r},{r} 0 0 1 {r},{-r}z"
    )


def rect(x, y, w, h):
    return f"M{x},{y}h{w}v{h}h{-w}z"


def frame(x, y, w, h, r, t=1.0):
    """Outline of thickness `t` as an even-odd path (outer + inner)."""
    return rrect(x, y, w, h, r) + rrect(x + t, y + t, w - 2 * t, h - 2 * t, max(r - t, 0.01))


KEYS_ROW = [2.5, 5.5, 8.5, 11.5]


def keyboard(keys=True, space=True):
    d = frame(0, 0.5, 16, 9.5, 1.5)
    if keys:
        for x in KEYS_ROW:
            d += rect(x, 2.5, 2, 1.5)
            d += rect(x, 5, 2, 1.5)
    if space:
        d += rect(4.5, 7.5, 7, 1)
    return d


def gauge_frame():
    return frame(0, 11, 16, 5, 1.5)


def gauge_fill(level):
    """Fill of the gauge, 0..100 -> 0..12.5 px (0.5 px gap inside the frame)."""
    w = round(12.5 * level / 100, 2)
    return rect(1.75, 12.5, w, 2) if w > 0 else ""


def path(d, cls="ColorScheme-Text", extra_class=""):
    c = f"{cls} {extra_class}".strip()
    return f'<path class="{c}" d="{d}" fill="currentColor" fill-rule="evenodd"/>'


BOLT = "M9.5,1.5L5.5,6h2.25L6.5,9.5L10.5,5H8.25z"


def svg(*paths):
    body = "".join(paths)
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">'
        f"{STYLE}{body}</svg>\n"
    )


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    files = {}
    for level in range(0, 101, 10):
        name = f"apihub-kb-battery-{level:03d}"
        files[f"{name}-symbolic.svg"] = svg(
            path(keyboard() + gauge_frame() + gauge_fill(level))
        )
        # Charging: keys of the middle replaced by a bolt.
        files[f"{name}-charging-symbolic.svg"] = svg(
            path(frame(0, 0.5, 16, 9.5, 1.5) + gauge_frame() + gauge_fill(level)),
            path(BOLT, "ColorScheme-PositiveText", "success"),
        )
    files["apihub-kb-battery-caution-symbolic.svg"] = svg(
        path(keyboard()),
        path(gauge_frame() + rect(1.75, 12.5, 1.5, 2), "ColorScheme-NegativeText", "error"),
    )
    # Disconnected: keyboard crossed out, no gauge.
    slash = "M1.5,15.5L0.5,14.5L14.5,0.5L15.5,1.5z"
    files["apihub-kb-disconnected-symbolic.svg"] = svg(
        path(frame(0, 0.5, 16, 9.5, 1.5) + rect(4.5, 7.5, 7, 1)),
        path(slash),
    )
    # Missing: dashed outline (no keyboard known / no source).
    dashes = ""
    for x in (1.5, 5.5, 9.5, 13.5):
        dashes += rect(x, 2, 1.5, 1) + rect(x, 13, 1.5, 1)
    for y in (4.5, 8.5):
        dashes += rect(0.5, y, 1, 2) + rect(14.5, y, 1, 2)
    files["apihub-kb-missing-symbolic.svg"] = svg(path(dashes))

    for n, content in files.items():
        (OUT / n).write_text(content)
    print(f"{len(files)} icons written to {OUT}")


if __name__ == "__main__":
    main()
