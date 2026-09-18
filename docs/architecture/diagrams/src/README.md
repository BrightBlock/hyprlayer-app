# Diagram sources

The SVGs one directory up are generated, not hand-drawn. Regenerate them after
editing any generator here:

```bash
cd docs/architecture/diagrams/src
python3 gen_sequence.py && python3 gen_main.py && python3 gen_threads.py
python3 gen_memory.py
```

Each generator writes its `.dc.html` beside itself, and `gen_memory.py` writes the
three project-memory boards. The committed `.svg` is that file's `<svg>` element
with an XML declaration in front of it:

```bash
python3 - <<'PY'
import re, pathlib
for src, dst in [("Sequence.dc.html", "../sequence.svg"),
                 ("Main.dc.html", "../architecture.svg"),
                 ("Threads.dc.html", "../terminal-threads.svg"),
                 ("MemorySequence.dc.html", "../memory-sequence.svg"),
                 ("MemoryMain.dc.html", "../memory-architecture.svg"),
                 ("MemoryCoverage.dc.html", "../memory-coverage.svg")]:
    svg = re.search(r"(<svg\b.*?</svg>)", pathlib.Path(src).read_text(), re.S).group(1)
    pathlib.Path(dst).write_text('<?xml version="1.0" encoding="UTF-8"?>\n' + svg + "\n")
PY
```

`kit.py` holds the palette — hyprlayer's own `src/index.css` dark tokens — and the
text-width estimators the layout arithmetic depends on. The published fonts (Inter,
JetBrains Mono) are not embedded, so a standalone SVG falls back to the system sans
and mono; the estimators are close enough to both that nothing reflows.

The `.dc.html` intermediates are Claude Design artboards and are deliberately not
committed — only the generators and the rendered SVGs are.
