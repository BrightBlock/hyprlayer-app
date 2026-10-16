from kit import *

W = 1600
out = []


def card(x, y, w, title, lines, colour, tint=0.0, rx=9):
    """A titled box; `lines` are 11.5px mono detail rows. Returns (svg, height)."""
    pad = 14
    h = pad + 18 + len(lines) * 17 + pad - 2
    s = [rect(x, y, w, h, fill=PANEL2, stroke=EDGE, rx=rx)]
    if tint:
        s.append(rect(x, y, w, h, fill=colour, stroke="none", rx=rx, opacity=tint))
    s.append(rect(x, y + 10, 3, h - 20, fill=colour, stroke="none", rx=2))
    s.append(text(x + pad + 8, y + pad + 12, title, size=13.5, fill=INK,
                  family=SANS, weight="600", anchor="start"))
    for i, ln in enumerate(lines):
        s.append(text(x + pad + 8, y + pad + 12 + 20 + i * 17, ln, size=11.5,
                      fill=DIM, family=MONO, anchor="start"))
    return "".join(s), h


def panel(x, y, w, h, title, sub, colour):
    s = [rect(x, y, w, h, fill=PANEL, stroke=EDGE, rx=14),
         rect(x, y, w, 3, fill=colour, stroke="none", rx=2),
         text(x + 24, y + 36, title, size=17, fill=INK, family=SANS,
              weight="600", anchor="start"),
         text(x + 24, y + 57, sub, size=12, fill=DIM, family=SANS, anchor="start")]
    return "".join(s)


out.append(svg_open(W, 1230))
out.append(defs({"blue": BLUE, "accent": ACCENT, "teal": TEAL, "violet": VIOLET,
                 "dim": DIM}))

out.append(text(48, 56, "One runtime, two thin shells", size=30, fill=INK,
                family=SANS, weight="600", anchor="start"))
out.append(text(48, 84, "ADR-0009 — the whole runtime lives in a supervised hyprlayer-server process. The desktop is always a client: locally over a socket, remotely over SSH.",
                size=14.5, fill=DIM, family=SANS, anchor="start"))

TOP, PH = 118, 706
CX, CW = 40, 440
WX, WW = 524, 336
SX, SW = 904, 656

# ── Client ──────────────────────────────────────────────────────────────
out.append(panel(CX, TOP, CW, PH, "Client machine", "Hyprlayer.app — holds no runtime of its own", BLUE))
y = TOP + 78
for title, lines, colour in [
    ("Webview", ["React 19 — routes · features · lib"], BLUE),
    ("src-tauri commands", ["172 #[tauri::command] bodies,",
                            "most of them one RPC call"], BLUE),
    ("AppState", ["one field: the RpcClient",
                  "Connected · Lost · Failed"], BLUE),
    ("supervisor.rs", ["connect · spawn · evict a stale server",
                       "classify_ssh_failure"], ACCENT),
    ("provision.rs", ["installs ~/.hyprlayer/server/<version>/",
                      "on the host, over ssh + curl"], ACCENT),
    ("hosts.json", ["which server to reach — decided locally,",
                    "because there is no server to ask yet"], ACCENT),
    ("Client-only carve-outs", ["editors · reveal_doc · detect_editors",
                                "import_attachment · sound · FDA · updater",
                                "remote → CoreError::RemoteUnsupported"], BLUE),
]:
    s, h = card(CX + 22, y, CW - 44, title, lines, colour)
    out.append(s)
    y += h + 11

# ── Transport ───────────────────────────────────────────────────────────
out.append(panel(WX, TOP, WW, PH, "Transport", "hyprlayer-proto — serde only, no TypeScript", ACCENT))
y = TOP + 78
for title, lines, colour in [
    ("Local", ["unix socket / named pipe",
               "$XDG_RUNTIME_DIR/hyprlayer/<hash>.sock",
               "0600 inside a 0700 directory"], ACCENT),
    ("Remote", ["ssh <destination>",
                "  ~/.hyprlayer/server/<version>/",
                "    hyprlayer-server relay"], ACCENT),
    ("Frames", ["'C' + JSON line",
                "'D' + u32 len + payload",
                "payload = u64 stream id ++ event"], ACCENT),
    ("Control plane", ["Hello · Refused · Shutdown",
                       "Request · Response · Event",
                       "StreamRequest · StreamResponse",
                       "Cancel"], ACCENT),
    ("PROTOCOL_VERSION", ["strict equality on the handshake —",
                          "a newer client evicts an older server"], ACCENT),
]:
    s, h = card(WX + 20, y, WW - 40, title, lines, colour)
    out.append(s)
    y += h + 11

# ── Server ──────────────────────────────────────────────────────────────
out.append(panel(SX, TOP, SW, PH, "Server machine", "hyprlayer-server — one runtime, one profile, no per-connection identity", TEAL))
y = TOP + 78
s, h = card(SX + 22, y, SW - 44, "SocketServer",
            ["accept loop — one task tree per connection, aborted on stop"], TEAL)
out.append(s); y += h + 11
s, h = card(SX + 22, y, SW - 44, "Per connection",
            ["a reader task · a writer task — the only writer to the socket",
             "one task per request · a StreamRegistry of abort handles"], TEAL)
out.append(s); y += h + 11

col_w = (SW - 44 - 22) / 2
s1, h1 = card(SX + 22, y, col_w, "dispatch.rs",
              ["one arm per command,", "CoreError → WireError"], TEAL)
s2, h2 = card(SX + 22 + col_w + 22, y, col_w, "streams.rs",
              ["prepare_stream mints the id,", "run_stream forwards the events"], TEAL)
out.append(s1); out.append(s2)
y += max(h1, h2) + 11
s, h = card(SX + 22, y, SW - 44, "events.rs",
            ["one subscription to the runtime's broadcast, fanned out to every client"], TEAL)
out.append(s); y += h + 11

fh = 128
out.append(rect(SX + 22, y, SW - 44, fh, fill=PANEL2, stroke=TEAL, rx=9))
out.append(rect(SX + 22, y, SW - 44, fh, fill=TEAL, stroke="none", rx=9, opacity=0.07))
out.append(text(SX + 44, y + 26, "HyprlayerRuntime", size=15, fill=INK,
                family=SANS, weight="600", anchor="start"))
out.append(text(SX + 44 + sans_w("HyprlayerRuntime", 15) * 1.08 + 14, y + 26,
                "the supported facade — ADR-0008", size=11.5, fill=DIM,
                family=SANS, anchor="start"))
svc = ["settings", "providers", "sessions", "projects", "library", "agents",
       "prompts", "workstreams", "attachments", "terminals", "scripts"]
sx, sy = SX + 44, y + 50
for name in svc:
    cw = mono_w(name + "()", 11.5) + 20
    if sx + cw > SX + SW - 44:
        sx, sy = SX + 44, sy + 30
    out.append(rect(sx, sy, cw, 23, fill=SURFACE, stroke=EDGE, rx=6))
    out.append(text(sx + cw / 2, sy + 16, name + "()", size=11.5, fill=TEAL, family=MONO))
    sx += cw + 9
y += fh + 11
s, h = card(SX + 22, y, SW - 44, "ProfileLock",
            ["one runtime per app_data_dir — one settings.json, one hyprlayer.db, one session set"], TEAL)
out.append(s)

# ── Connectors ──────────────────────────────────────────────────────────
for y0 in (TOP + 250, TOP + 300):
    pass
out.append(line(CX + CW + 6, TOP + 270, WX - 10, TOP + 270, stroke=ACCENT, sw=2, marker="accent"))
out.append(line(WX - 10, TOP + 310, CX + CW + 6, TOP + 310, stroke=ACCENT, sw=2,
                marker="accent", dash="6 5"))
out.append(line(WX + WW + 6, TOP + 270, SX - 10, TOP + 270, stroke=ACCENT, sw=2, marker="accent"))
out.append(line(SX - 10, TOP + 310, WX + WW + 6, TOP + 310, stroke=ACCENT, sw=2,
                marker="accent", dash="6 5"))

# ── Row B: crates, and what the server owns ─────────────────────────────
BY = TOP + PH + 34
out.append(rect(CX, BY, CW + 20 + WW + 44, 188, fill=PANEL, stroke=EDGE, rx=14))
out.append(text(CX + 24, BY + 34, "Crate layering", size=17, fill=INK, family=SANS,
                weight="600", anchor="start"))
out.append(text(CX + 24, BY + 55, "cargo rejects the cycle, so the direction needs no other rule",
                size=12, fill=DIM, family=SANS, anchor="start"))
chain = [("hyprlayer-models", "serde contracts only", DIM),
         ("hyprlayer-core", "all logic · the facade", TEAL),
         ("hyprlayer-server", "the lock · the socket", TEAL),
         ("hyprlayer-app", "RPC client · desktop", BLUE)]
cw = 176
cx = CX + 26
for i, (name, sub, colour) in enumerate(chain):
    out.append(rect(cx, BY + 78, cw, 56, fill=PANEL2, stroke=EDGE, rx=9))
    out.append(rect(cx, BY + 78, cw, 3, fill=colour, stroke="none", rx=2))
    out.append(text(cx + cw / 2, BY + 102, name, size=12.5, fill=INK, family=MONO, weight="500"))
    out.append(text(cx + cw / 2, BY + 121, sub, size=11, fill=DIM, family=SANS))
    if i < 3:
        out.append(line(cx + cw + 6, BY + 106, cx + cw + 24, BY + 106,
                        stroke=DIM, sw=1.6, marker="dim"))
    cx += cw + 30
out.append(rect(CX + 26, BY + 150, 176, 26, fill=PANEL2, stroke=EDGE, rx=7))
out.append(text(CX + 26 + 88, BY + 168, "hyprlayer-proto", size=11.5, fill=ACCENT, family=MONO))
out.append(path(f"M {CX + 26 + 176 + 6} {BY + 163} H {CX + 26 + 2 * (176 + 30) + 88} V {BY + 140}",
                stroke=ACCENT, sw=1.4, dash="5 4", marker="accent"))
out.append(path(f"M {CX + 26 + 176 + 6} {BY + 163} H {CX + 26 + 3 * (176 + 30) + 88} V {BY + 140}",
                stroke=ACCENT, sw=1.4, dash="5 4", marker="accent"))

out.append(rect(SX, BY, SW, 188, fill=PANEL, stroke=EDGE, rx=14))
out.append(rect(SX, BY, SW, 3, fill=VIOLET, stroke="none", rx=2))
out.append(text(SX + 24, BY + 34, "What the server owns, on its own machine", size=17,
                fill=INK, family=SANS, weight="600", anchor="start"))
out.append(text(SX + 24, BY + 55, "everything the desktop used to touch directly",
                size=12, fill=DIM, family=SANS, anchor="start"))
res = [("hyprlayer.db", "SQLite — sessions,\nworkstreams, the log"),
       ("settings.json", "and the hyprlayer\nCLI config it reads"),
       ("worktrees", "git2 reads,\ngit + gh shell-outs"),
       ("PTYs", "terminals, scripts,\nagent shells"),
       ("harness CLIs", "claude · codex\nopencode serve")]
rw = (SW - 48 - 4 * 12) / 5
rx_ = SX + 24
for name, sub in res:
    out.append(rect(rx_, BY + 76, rw, 92, fill=PANEL2, stroke=EDGE, rx=9))
    out.append(rect(rx_, BY + 76, rw, 3, fill=VIOLET, stroke="none", rx=2))
    out.append(text(rx_ + rw / 2, BY + 102, name, size=12, fill=INK, family=MONO, weight="500"))
    for i, ln in enumerate(sub.split("\n")):
        out.append(text(rx_ + rw / 2, BY + 124 + i * 16, ln, size=10, fill=DIM, family=SANS))
    rx_ += rw + 12

# ── Row C: the three facts the boxes cannot show ────────────────────────
NY = BY + 188 + 26
facts = [("The environment overlay", "A daemonised server has no SSH_AUTH_SOCK, so every attach sends one and the server merges it at each child spawn. Last attach wins — except opencode serve, which bakes it in and keeps the first.", ACCENT),
         ("Version skew is eviction, not negotiation", "A newer client asks an older server to shut down, waits for the profile lock, and spawns its own — which ends every live session on that profile.", ACCENT),
         ("A failed connect still opens the window", "Into a recovery state that retries, runtime_in_use included: it shows the error and its remedy.", ACCENT)]
fw = (W - 80 - 2 * 20) / 3
fx = 40
for title, body, colour in facts:
    out.append(rect(fx, NY, fw, 108, fill=PANEL, stroke=EDGE, rx=10))
    out.append(rect(fx, NY + 12, 3, 84, fill=colour, stroke="none", rx=2))
    out.append(text(fx + 22, NY + 28, title, size=13, fill=INK, family=SANS,
                    weight="600", anchor="start"))
    # wrap
    words, lines, cur = body.split(" "), [], ""
    for wd in words:
        trial = (cur + " " + wd).strip()
        if sans_w(trial, 11.5) > fw - 44:
            lines.append(cur)
            cur = wd
        else:
            cur = trial
    lines.append(cur)
    for i, ln in enumerate(lines[:5]):
        out.append(text(fx + 22, NY + 50 + i * 16, ln, size=11.5, fill=DIM,
                        family=SANS, anchor="start"))
    fx += fw + 20

out.append("</svg>")
write_doc("Main.dc.html", "".join(out))
print(f"Main.dc.html  {W}x1230")
