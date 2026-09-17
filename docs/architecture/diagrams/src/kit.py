"""Shared SVG helpers. Palette is hyprlayer's own src/index.css dark theme."""

SURFACE = "#0c0c0c"
PANEL = "#141414"
PANEL2 = "#1c1c1c"
EDGE = "#333333"
INK = "#e8e6e2"
DIM = "#a8a6a2"
ACCENT = "#d97757"   # the wire
BLUE = "#60a5fa"     # client
TEAL = "#2dd4bf"     # server
VIOLET = "#a78bfa"   # child processes
BAD = "#ef5350"

SANS = "'Inter','Inter Variable',system-ui,-apple-system,'Segoe UI',sans-serif"
MONO = "'JetBrains Mono','JetBrains Mono Variable',ui-monospace,SFMono-Regular,Menlo,monospace"

MONO_W = 0.60   # advance width per px of font-size
SANS_W = 0.545


def esc(s):
    return (s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
             .replace('"', "&quot;"))


def mono_w(text, size):
    return len(text) * size * MONO_W


def sans_w(text, size):
    return len(text) * size * SANS_W


def defs(colors):
    """One arrowhead marker per colour, plus the two dash patterns."""
    out = ["<defs>"]
    for name, colour in colors.items():
        out.append(
            f'<marker id="ah-{name}" viewBox="0 0 10 10" refX="9.2" refY="5" '
            f'markerWidth="7" markerHeight="7" orient="auto-start-reverse">'
            f'<path d="M 0 1.2 L 9.4 5 L 0 8.8 z" fill="{colour}"/></marker>'
        )
    out.append("</defs>")
    return "".join(out)


def text(x, y, s, size=13, fill=INK, family=MONO, weight="400", anchor="middle",
         opacity=None, spacing=None):
    extra = ""
    if opacity is not None:
        extra += f' opacity="{opacity}"'
    if spacing is not None:
        extra += f' letter-spacing="{spacing}"'
    return (f'<text x="{x:.1f}" y="{y:.1f}" text-anchor="{anchor}" '
            f'style="font-family:{family};font-size:{size}px;font-weight:{weight};'
            f'fill:{fill}"{extra}>{esc(s)}</text>')


def rect(x, y, w, h, fill=PANEL, stroke=EDGE, rx=10, sw=1, dash=None, opacity=None):
    d = f' stroke-dasharray="{dash}"' if dash else ""
    o = f' opacity="{opacity}"' if opacity is not None else ""
    return (f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" rx="{rx}" '
            f'style="fill:{fill};stroke:{stroke};stroke-width:{sw}"{d}{o}/>')


def line(x1, y1, x2, y2, stroke=EDGE, sw=1, dash=None, marker=None, cap="round"):
    d = f' stroke-dasharray="{dash}"' if dash else ""
    m = f' marker-end="url(#ah-{marker})"' if marker else ""
    return (f'<line x1="{x1:.1f}" y1="{y1:.1f}" x2="{x2:.1f}" y2="{y2:.1f}" '
            f'style="stroke:{stroke};stroke-width:{sw};stroke-linecap:{cap}"{d}{m}/>')


def path(d, stroke=EDGE, sw=1.4, fill="none", dash=None, marker=None):
    ds = f' stroke-dasharray="{dash}"' if dash else ""
    m = f' marker-end="url(#ah-{marker})"' if marker else ""
    return (f'<path d="{d}" style="fill:{fill};stroke:{stroke};stroke-width:{sw};'
            f'stroke-linecap:round;stroke-linejoin:round"{ds}{m}/>')


def label_on_line(cx, y, s, size=12.5, fill=INK, family=MONO, weight="400",
                  bg=SURFACE, pad=7):
    """A message label with the lifeline masked out behind it."""
    w = (mono_w(s, size) if family == MONO else sans_w(s, size)) + pad * 2
    return (rect(cx - w / 2, y - size + 1.5, w, size + 5, fill=bg, stroke="none", rx=3)
            + text(cx, y, s, size=size, fill=fill, family=family, weight=weight))


def chip(x, y, s, colour, size=11, weight="600"):
    """A small uppercase tag, left-aligned at x, baseline y."""
    w = sans_w(s.upper(), size) * 1.08 + 18
    return (rect(x, y - size - 3, w, size + 10, fill=colour, stroke="none", rx=5, opacity=0.14)
            + text(x + w / 2, y, s.upper(), size=size, fill=colour, family=SANS,
                   weight=weight, spacing="0.07em"))


def svg_open(w, h, bg=SURFACE):
    return (f'<svg viewBox="0 0 {w} {h}" width="{w}" height="{h}" '
            f'xmlns="http://www.w3.org/2000/svg" style="display:block">'
            f'<rect x="0" y="0" width="{w}" height="{h}" style="fill:{bg}"/>')


DOC = """<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <script src="./support.js"></script>
</head>
<body>
<x-dc>
<helmet>
  <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap">
  <style>
    body {{ margin: 0; background: {surface}; }}
    a {{ color: {accent}; }}
    a:hover {{ color: #e89272; }}
  </style>
</helmet>
{body}
</x-dc>
</body>
</html>
"""


def write_doc(filename, body):
    with open(filename, "w") as f:
        f.write(DOC.format(surface=SURFACE, accent=ACCENT, body=body))
