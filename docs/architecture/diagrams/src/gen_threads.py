from kit import *

W, H = 1600, 1028
out = []


def unit(x, y, w, tag, title, lines, colour, kind="thread", h=None):
    """A box. kind: thread = solid coloured, task = dashed, plain = neither."""
    pad = 13
    hh = h or (pad + 13 + 19 + len(lines) * 16 + pad - 4)
    stroke = EDGE if kind == "plain" else colour
    s = [rect(x, y, w, hh, fill=PANEL2, stroke=stroke, rx=9,
              dash="5 4" if kind == "task" else None)]
    if kind != "plain":
        s.append(rect(x, y, w, hh, fill=colour, stroke="none", rx=9, opacity=0.06))
    if tag:
        s.append(text(x + pad, y + pad + 8, tag.upper(), size=9.5, fill=colour,
                      family=SANS, weight="600", anchor="start", spacing="0.1em"))
    s.append(text(x + pad, y + pad + (26 if tag else 12), title, size=13.5, fill=INK,
                  family=SANS, weight="600", anchor="start"))
    for i, ln in enumerate(lines):
        s.append(text(x + pad, y + pad + (46 if tag else 32) + i * 16, ln, size=11,
                      fill=DIM, family=MONO, anchor="start"))
    return "".join(s), hh


def pill(cx, cy, s, colour=ACCENT, size=10.5):
    w = mono_w(s, size) + 18
    return (rect(cx - w / 2, cy - 11, w, 22, fill=SURFACE, stroke=colour, rx=11)
            + text(cx, cy + 4, s, size=size, fill=colour, family=MONO))


def band(y, name, sub, colour):
    return "".join([
        rect(40, y, W - 80, 32, fill=PANEL, stroke="none", rx=8),
        rect(40, y, 3, 32, fill=colour, stroke="none", rx=2),
        text(62, y + 21, name, size=13.5, fill=INK, family=SANS, weight="600", anchor="start"),
        text(62 + sans_w(name, 13.5) * 1.08 + 18, y + 21, sub, size=12.5, fill=DIM,
             family=SANS, anchor="start")])


def zone(x, y, w, h, title, sub, colour):
    return (rect(x, y, w, h, fill=PANEL, stroke=EDGE, rx=13)
            + rect(x, y, w, 3, fill=colour, stroke="none", rx=2)
            + text(x + 18, y + 30, title, size=14.5, fill=INK, family=SANS,
                   weight="600", anchor="start")
            + text(x + 18, y + 49, sub, size=11, fill=DIM, family=SANS, anchor="start"))


out.append(svg_open(W, H))
out.append(defs({"blue": BLUE, "accent": ACCENT, "teal": TEAL, "violet": VIOLET,
                 "dim": DIM, "ink": INK}))

out.append(text(48, 56, "One terminal, three threads, many cursors", size=30, fill=INK,
                family=SANS, weight="600", anchor="start"))
out.append(text(48, 84, "Blocking pty I/O stays on OS threads, so a slow reader backpressures the shell through the kernel's own buffer instead of growing a queue.",
                size=14.5, fill=DIM, family=SANS, anchor="start"))

# Legend — the border carries the meaning, so the swatches are colourless
lx = 880
for dash, lab in ((None, "OS thread"), ("5 4", "tokio task")):
    out.append(rect(lx, 46, 26, 16, fill=PANEL2, stroke=DIM, rx=5, dash=dash))
    out.append(text(lx + 34, 58, lab, size=12, fill=DIM, family=SANS, anchor="start"))
    lx += 34 + sans_w(lab, 12) * 1.06 + 26
out.append(rect(lx, 46, 26, 16, fill=PANEL2, stroke=EDGE, rx=5))
out.append(text(lx + 34, 58, "neither — a value or a channel", size=12, fill=DIM,
                family=SANS, anchor="start"))
lx += 34 + sans_w("neither — a value or a channel", 12) * 1.06 + 26
out.append(pill(lx + 58, 54, "channel(cap)", colour=ACCENT))

# ── Row 1: the output path ──────────────────────────────────────────────
out.append(band(112, "Output", "one copy of the bytes, one cursor per subscriber", VIOLET))

RY, RH = 168, 470
out.append(zone(40, RY, 160, RH, "Child", "shell · script · agent", VIOLET))
out.append(rect(58, RY + 200, 124, 76, fill=PANEL2, stroke=VIOLET, rx=9))
out.append(rect(58, RY + 200, 124, 76, fill=VIOLET, stroke="none", rx=9, opacity=0.06))
out.append(text(120, RY + 230, "pty slave", size=12.5, fill=INK, family=MONO))
out.append(text(120, RY + 250, "$SHELL -l", size=11, fill=DIM, family=MONO))

PX, PW = 226, 604
out.append(zone(PX, RY, PW, RH, "PtySession", "spawned once per terminal — terminal/mod.rs:212", VIOLET))
tw = 312
s, h1 = unit(PX + 18, RY + 66, tw, "OS thread 1 · reader",
             "read() 8 KiB at a time", ["from the pty master"], VIOLET)
out.append(s)
y2 = RY + 66 + h1 + 44
s, h2 = unit(PX + 18, y2, tw, "OS thread 2 · coalescer",
             "flush at 64 KiB", ["hold back a torn UTF-8 tail, so", "every flush end is a char boundary"], VIOLET)
out.append(s)
y3 = y2 + h2 + 44
s, h3 = unit(PX + 18, y3, tw, "OS thread 3 · child waiter",
             "child.wait()", ["then drop the pty master"], VIOLET)
out.append(s)

out.append(line(PX + 18 + tw / 2, RY + 66 + h1, PX + 18 + tw / 2, y2, stroke=ACCENT,
                sw=1.6, marker="accent"))
out.append(pill(PX + 18 + tw / 2, RY + 66 + h1 + 22, "sync_channel(16)"))
out.append(line(PX + 18 + tw / 2, y3, PX + 18 + tw / 2, y2 + h2, stroke=DIM,
                sw=1.5, marker="dim"))
out.append(pill(PX + 18 + tw / 2, y3 - 22, "exit code · std mpsc", colour=DIM))

RGX, RGW = PX + 18 + tw + 26, PW - 18 - tw - 26 - 18
rgy, rgh = RY + 76, 300
out.append(rect(RGX, rgy, RGW, rgh, fill=SURFACE, stroke=ACCENT, rx=10))
out.append(rect(RGX, rgy, RGW, rgh, fill=ACCENT, stroke="none", rx=10, opacity=0.05))
out.append(text(RGX + 16, rgy + 26, "OutputRing", size=14, fill=INK, family=SANS,
                weight="600", anchor="start"))
out.append(text(RGX + 16, rgy + 44, "the one copy of the bytes", size=11, fill=DIM,
                family=SANS, anchor="start"))
for i, ln in enumerate(["Mutex<RingInner>", "  VecDeque, cap 256 KiB", "  head", "  snapshot_floor",
                        "  flush_ends", "  exited", "", "watch::Sender<tail>",
                        "  the wakeup for", "  every reader"]):
    out.append(text(RGX + 16, rgy + 74 + i * 17, ln,
                    size=11, fill=INK if not ln.startswith("  ") else DIM,
                    family=MONO, anchor="start"))
for i, ln in enumerate(["a trim drops back to a flush", "end, so head always lands on",
                        "a char boundary"]):
    out.append(text(RGX + 16, rgy + rgh - 50 + i * 15, ln, size=10.5, fill=ACCENT,
                    family=SANS, anchor="start"))

out.append(line(PX + 18 + tw + 6, y2 + h2 / 2, RGX - 8, y2 + h2 / 2, stroke=ACCENT,
                sw=1.8, marker="accent"))
out.append(line(200 + 6, RY + 238, PX - 8, RY + 238, stroke=VIOLET, sw=1.8, marker="violet"))

# ── Per attached client ────────────────────────────────────────────────
AX, AW = 856, 320
out.append(zone(AX, RY, AW, RH, "Per attached client", "two tokio tasks, and a cursor", TEAL))
ay = RY + 66
s, h = unit(AX + 18, ay, AW - 36, None, "RingReader",
            ["cursor + watch::Receiver", "base64-encodes outside the lock",
             "below head = lagged, resumes", "at head with a gap"], TEAL, kind="plain")
out.append(s)
ay += h + 40
out.append(pill(AX + AW / 2, ay - 20, "recv()", colour=TEAL))
s, h = unit(AX + 18, ay, AW - 36, "tokio task", "spawn_forward",
            ["hand each event to the attach", "queue, unregister on Exited"], TEAL, kind="task")
out.append(s)
ay += h + 40
out.append(pill(AX + AW / 2, ay - 20, "mpsc(256)"))
s, h = unit(AX + 18, ay, AW - 36, "tokio task", "run_stream",
            ["frame as", "Data { stream_id, event }"], TEAL, kind="task")
out.append(s)

out.append(path(f"M {RGX + RGW + 8} {rgy + rgh / 2} H {AX - 8}", stroke=ACCENT, sw=1.8,
                marker="accent"))

# ── Per connection ─────────────────────────────────────────────────────
NX, NW = 1202, 184
out.append(zone(NX, RY, NW, RH, "Per connection", "shared by every stream", ACCENT))
ny = RY + 100
out.append(pill(NX + NW / 2, ny, "mpsc(1024)"))
s, h = unit(NX + 16, ny + 30, NW - 32, "tokio task", "writer",
            ["the only", "writer to", "the socket"], ACCENT, kind="task")
out.append(s)
out.append(line(AX + AW + 6, RY + 250, NX - 8, RY + 250, stroke=ACCENT, sw=1.8, marker="accent"))
out.append(rect(NX + 16, RY + 300, NW - 32, 118, fill=SURFACE, stroke=EDGE, rx=9))
out.append(rect(NX + 16, RY + 312, 3, 94, fill=ACCENT, stroke="none", rx=2))
for i, ln in enumerate(["StreamRegistry", "", "a dropped connection",
                        "aborts every stream", "it owned — and kills", "none of the PTYs"]):
    out.append(text(NX + 32, RY + 326 + i * 16, ln, size=10.5,
                    fill=INK if i == 0 else DIM,
                    family=MONO if i == 0 else SANS, weight="500" if i == 0 else "400",
                    anchor="start"))

# ── Client ─────────────────────────────────────────────────────────────
KX, KW = 1412, 148
out.append(zone(KX, RY, KW, RH, "Client", "the desktop", BLUE))
ky = RY + 66
for tag, title, lines, kind in [("tokio task", "read_loop", ["split by", "stream id"], "task"),
                                (None, "EventStream", ["unbounded", "per stream"], "plain"),
                                ("tokio task", "pump", ["forward to", "the Channel"], "task"),
                                (None, "xterm.js", ["Tauri", "Channel"], "plain")]:
    s, h = unit(KX + 14, ky, KW - 28, tag, title, lines, BLUE, kind=kind)
    out.append(s)
    if ky + h + 22 < RY + RH - 20:
        out.append(line(KX + KW / 2, ky + h, KX + KW / 2, ky + h + 20, stroke=BLUE,
                        sw=1.5, marker="blue"))
    ky += h + 22
out.append(line(NX + NW + 6, RY + 250, KX - 8, RY + 250, stroke=ACCENT, sw=1.8, marker="accent"))

# ── Row 2: the control path ────────────────────────────────────────────
CY = RY + RH + 34
out.append(band(CY, "Control", "the other direction — plain RPC, no threads of its own", ACCENT))
cy = CY + 50
steps = [("xterm.js", ["keystroke, resize,", "close"], BLUE, "plain"),
         ("terminalWrite · terminalResize", ["terminalClose — three plain", "commands, keyed by terminal_id"], ACCENT, "plain"),
         ("dispatch", ["one per request,", "on the server"], TEAL, "task"),
         ("TerminalRegistry", ["tokio Mutex<HashMap<", "  String, Arc<PtySession>>>"], TEAL, "plain"),
         ("PtySession handles", ["writer Mutex → master write", "master Mutex → resize",
                                 "killer + killpg(pgid)"], VIOLET, "plain")]
sw_ = (W - 80 - 4 * 46) / 5
sx = W - 40 - sw_
for i, (title, lines, colour, kind) in enumerate(steps):
    s, h = unit(sx, cy, sw_, None, title, lines, colour, kind=kind, h=92)
    out.append(s)
    if i < 4:
        out.append(line(sx - 8, cy + 46, sx - 38, cy + 46, stroke=DIM, sw=1.6, marker="dim"))
    sx -= sw_ + 46
out.append(path(f"M {34} {cy + 46} H {22} V {RY + 238} H {34}",
                stroke=VIOLET, sw=1.6, dash="5 4", marker="violet"))

# ── Row 3: consequences ────────────────────────────────────────────────
FY = cy + 92 + 48
facts = [("Replay and live output are the same bytes",
          "A new attach starts at snapshot_floor, so its first recv() is the retained scrollback and it streams on from there. No second buffer to keep in step."),
         ("Lag is bounded in bytes, not in messages",
          "A slow reader's slack is 256 KiB. Past that its cursor falls below head and resumes there, seeing a gap — never mid-codepoint, but possibly mid-line."),
         ("A dropped client is not a dropped terminal",
          "Closing a connection aborts run_stream and drops the RingReader. The three threads and the ring are untouched, which is what terminalAttach comes back to."),
         ("Only Exited unregisters",
          "spawn_forward removes the terminal from the registry when the child dies. terminalClose removes it and kills the whole process group.")]
fw = (W - 80 - 3 * 18) / 4
fx = 40
for title, body in facts:
    out.append(rect(fx, FY, fw, 124, fill=PANEL, stroke=EDGE, rx=10))
    out.append(rect(fx, FY + 12, 3, 100, fill=VIOLET, stroke="none", rx=2))
    words, lines, cur = title.split(" "), [], ""
    for wd in words:
        trial = (cur + " " + wd).strip()
        if sans_w(trial, 13) > fw - 40:
            lines.append(cur); cur = wd
        else:
            cur = trial
    lines.append(cur)
    ty = FY + 28
    for ln in lines:
        out.append(text(fx + 20, ty, ln, size=13, fill=INK, family=SANS,
                        weight="600", anchor="start"))
        ty += 17
    words, wrapped, cur = body.split(" "), [], ""
    for wd in words:
        trial = (cur + " " + wd).strip()
        if sans_w(trial, 11) > fw - 40:
            wrapped.append(cur); cur = wd
        else:
            cur = trial
    wrapped.append(cur)
    ty += 5
    for ln in wrapped[:6]:
        out.append(text(fx + 20, ty, ln, size=11, fill=DIM, family=SANS, anchor="start"))
        ty += 15
    fx += fw + 18

out.append("</svg>")
write_doc("Threads.dc.html", "".join(out))
print(f"Threads.dc.html  {W}x{H}   (last row ends at {FY + 124})")
