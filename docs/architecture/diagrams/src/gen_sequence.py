from kit import *

W = 1600
LX = [160, 480, 800, 1120, 1440]
BOUNDARY = [BLUE, ACCENT, TEAL, VIOLET]   # colour by the boundary a message crosses
ZONE = [BLUE, BLUE, TEAL, TEAL, VIOLET]   # colour by where a participant runs

PARTS = [
    ("Webview", "React 19"),
    ("Tauri shell", "AppState · RpcClient"),
    ("Server connection", "reader · writer · tasks"),
    ("HyprlayerRuntime", "the facade"),
    ("Child process", "PTY · claude · codex"),
]

ROW = 34
PHASE_H = 40

SCRIPT = [
    ("phase", "Connect", "supervisor.rs finds or spawns the profile's server"),
    ("msg", 1, 2, "connect  —  unix socket, or ssh <host> … relay stdio", None),
    ("msg", 2, 1, "Hello { protocol_version }", None),
    ("msg", 1, 2, "Hello { protocol_version }", None),
    ("note", 2, "strict equality — any other version is Refused, and only a newer client may evict"),
    ("msg", 1, 2, "SetEnvOverlay { SSH_AUTH_SOCK, DISPLAY, TERM, LANG, LC_* }", None),
    ("msg", 2, 3, "set_env_overlay", None),
    ("note", 3, "a daemonised server has none of these of its own"),
    ("msg", 1, 2, "WorktreeRoot", None),
    ("msg", 2, 1, "path  →  registers the asset scope", None),

    ("phase", "A plain command", "153 of them — one task per request, replies may come back out of order"),
    ("msg", 0, 1, "invoke listWorkstreams", None),
    ("msg", 1, 2, "Request { id, ListWorkstreams }", None),
    ("msg", 2, 3, "workstreams().list", None),
    ("msg", 3, 2, "Vec<Workstream>", None),
    ("msg", 2, 1, "Response { id, Ok }", None),
    ("msg", 1, 0, "typed result", None),

    ("phase", "A terminal", "a stream is opened once, then driven by plain commands on its id"),
    ("msg", 0, 1, "terminalOpen(workstreamId, cols, rows, Channel)", None),
    ("msg", 1, 2, "StreamRequest { id, TerminalOpen }", None),
    ("msg", 2, 3, "terminals().open", None),
    ("msg", 3, 4, "spawn PTY  —  login shell in the worktree", None),
    ("msg", 3, 2, "TerminalAttachment { terminal_id, live }", None),
    ("msg", 2, 1, "StreamResponse { terminal_id, stream_id }", None),
    ("note", 2, "the response reaches the wire before the forwarder starts, so no frame can precede the id that names it"),
    ("frame", "loop", "while the PTY lives"),
    ("msg", 4, 3, "output bytes", None),
    ("msg", 3, 2, "TerminalEvent::Output", None),
    ("msg", 2, 1, "Data frame { stream_id, event }", None),
    ("msg", 1, 0, "Channel message  →  xterm.js", None),
    ("endframe",),
    ("msg", 0, 1, "terminalWrite(terminal_id, base64)", None),
    ("msg", 1, 2, "Request { id, TerminalWrite }", None),
    ("msg", 2, 3, "terminals().write", None),
    ("msg", 3, 4, "pty master write", None),

    ("phase", "An agent", "the stream is opened first, the turn is launched second"),
    ("msg", 0, 1, "startStageSession(stage, input, Channel)", None),
    ("msg", 1, 2, "StreamRequest { id, StartStageSession }", None),
    ("msg", 2, 3, "sessions().prepare_start  +  attach", None),
    ("msg", 3, 2, "Session, prompt, live receiver", None),
    ("msg", 2, 1, "StreamResponse { session, prompt, stream_id }", None),
    ("msg", 0, 1, "launchAndConverse(session_id)", None),
    ("msg", 1, 2, "Request { id, LaunchAndConverse }", None),
    ("msg", 2, 3, "take the stashed launch params, launch", None),
    ("msg", 3, 4, "spawn claude / codex / opencode", None),
    ("frame", "loop", "turn in flight"),
    ("msg", 4, 3, "harness stdout  →  SessionEvent", None),
    ("msg", 3, 2, "persisted to SQLite, then fanned out", None),
    ("msg", 2, 1, "Data frame { stream_id, event }", None),
    ("msg", 1, 0, "Channel message  →  transcript", None),
    ("endframe",),
    ("msg", 0, 1, "answerQuestion / respondPermission / interruptSession", None),
    ("msg", 1, 2, "Request { id, … }", None),
    ("msg", 2, 3, "sessions().*", None),
    ("msg", 3, 4, "stdin write", None),
    ("frame", "opt", "the stream falls behind"),
    ("msg", 3, 2, "SessionStreamError::Lagged { epoch, last_persisted_seq }", None),
    ("msg", 2, 3, "log_after  →  re-send  →  re-attach", None),
    ("note", 2, "recovery is server-side — the client is never told a lag happened"),
    ("endframe",),

    ("phase", "Runtime events", "one subscription to the runtime, fanned out to every attached client"),
    ("msg", 3, 2, "RuntimeEvent  (broadcast)", None),
    ("msg", 2, 1, "Event { event }", None),
    ("msg", 1, 0, "Tauri event  →  react-query invalidation", None),

    ("phase", "Disconnect and reconnect", "the sessions and terminals keep running on the far side"),
    ("msg", 2, 1, "connection closed", BAD),
    ("msg", 1, 0, "serverDisconnected  ·  connectionStatus = Lost", BLUE),
    ("msg", 0, 1, "reconnectServer  —  1s, 2s, 5s, 15s, then 30s, and on focus", None),
    ("msg", 1, 2, "connect  +  Hello  +  SetEnvOverlay", None),
    ("msg", 0, 1, "terminalAttach / attachSession on the ids it kept", None),
    ("msg", 2, 1, "fresh stream ids over the same live PTYs and sessions", None),
]


def build():
    out = []
    head_top, head_h = 112, 66
    y = head_top + head_h + 30
    rows = []          # deferred body ops, drawn above the lifelines
    frames = []        # (label, sub, y0, y1)
    open_frame = None

    for op in SCRIPT:
        kind = op[0]
        if kind == "phase":
            y += 20
            rows.append(("phase", y, op[1], op[2]))
            y += PHASE_H
        elif kind == "msg":
            _, a, b, s, override = op
            rows.append(("msg", y, a, b, s, override))
            y += ROW
        elif kind == "note":
            rows.append(("note", y, op[1], op[2]))
            y += 28
        elif kind == "frame":
            y += 8
            open_frame = (op[1], op[2], y)
            y += 30
        elif kind == "endframe":
            y += 10
            frames.append((open_frame[0], open_frame[1], open_frame[2], y))
            open_frame = None
            y += 12

    bottom = y + 26
    height = bottom + 66

    out.append(svg_open(W, height))
    out.append(defs({"blue": BLUE, "accent": ACCENT, "teal": TEAL,
                     "violet": VIOLET, "bad": BAD, "dim": DIM}))

    # Heading
    out.append(text(48, 56, "Client ↔ server", size=30, fill=INK, family=SANS,
                    weight="600", anchor="start"))
    out.append(text(48, 84, "Three request shapes on one connection: request/response, streams, and unsolicited runtime events.",
                    size=14.5, fill=DIM, family=SANS, anchor="start"))

    # Lifelines, drawn first so everything else sits on top
    for x in LX:
        out.append(line(x, head_top + head_h, x, bottom, stroke=EDGE, sw=1, dash="3 6"))

    # Frames
    for lbl, sub, y0, y1 in frames:
        out.append(rect(108, y0, 1500 - 108 + 60, y1 - y0, fill="none",
                        stroke=EDGE, rx=8, dash="6 6"))
        tab_w = sans_w(lbl, 11) * 1.1 + 22
        out.append(rect(108, y0, tab_w, 22, fill=PANEL2, stroke=EDGE, rx=6))
        out.append(text(108 + tab_w / 2, y0 + 15, lbl, size=11, fill=DIM,
                        family=SANS, weight="600", spacing="0.06em"))
        out.append(text(108 + tab_w + 12, y0 + 15, sub, size=12, fill=DIM,
                        family=SANS, anchor="start"))

    # Rows
    for row in rows:
        if row[0] == "phase":
            _, ry, name, sub = row
            out.append(rect(40, ry - 2, W - 80, PHASE_H - 8, fill=PANEL,
                            stroke="none", rx=8))
            out.append(rect(40, ry - 2, 3, PHASE_H - 8, fill=ACCENT, stroke="none", rx=2))
            out.append(text(62, ry + 19, name, size=13.5, fill=INK, family=SANS,
                            weight="600", anchor="start"))
            out.append(text(62 + sans_w(name, 13.5) * 1.08 + 18, ry + 19, sub,
                            size=12.5, fill=DIM, family=SANS, anchor="start"))
        elif row[0] == "note":
            _, ry, idx, s = row
            x0 = LX[idx] - 300
            out.append(line(x0, ry + 6, x0, ry + 20, stroke=ACCENT, sw=2))
            out.append(text(x0 + 12, ry + 18, s, size=12, fill=DIM, family=SANS,
                            anchor="start"))
        else:
            _, ry, a, b, s, override = row
            colour = override or BOUNDARY[min(a, b)]
            name = {BLUE: "blue", ACCENT: "accent", TEAL: "teal",
                    VIOLET: "violet", BAD: "bad"}[colour]
            ly = ry + 24
            back = b < a
            x1, x2 = LX[a], LX[b]
            dash = "6 5" if back else None
            out.append(line(x1, ly, x2, ly, stroke=colour, sw=1.6, dash=dash,
                            marker=name))
            out.append(label_on_line((x1 + x2) / 2, ly - 8, s, size=12.5,
                                     fill=INK if not back else DIM))
            if colour == BAD:
                out.append(line(x2 - 7, ly - 7, x2 + 7, ly + 7, stroke=BAD, sw=2.2))
                out.append(line(x2 - 7, ly + 7, x2 + 7, ly - 7, stroke=BAD, sw=2.2))

    # Participant headers, on top of the lifelines
    for i, (name, sub) in enumerate(PARTS):
        bw = 250
        x = LX[i] - bw / 2
        out.append(rect(x, head_top, bw, head_h, fill=PANEL2, stroke=EDGE, rx=10))
        out.append(rect(x, head_top, bw, 3, fill=ZONE[i], stroke="none", rx=2))
        out.append(text(LX[i], head_top + 28, name, size=14.5, fill=INK,
                        family=SANS, weight="600"))
        out.append(text(LX[i], head_top + 48, sub, size=11.5, fill=DIM, family=SANS))
        out.append(rect(x + 70, bottom, 110, 12, fill=PANEL2, stroke=EDGE, rx=4))

    # Legend
    ly = height - 30
    out.append(text(48, ly, "solid = a call", size=12, fill=DIM, family=SANS, anchor="start"))
    out.append(text(190, ly, "dashed = a return or a pushed event", size=12, fill=DIM,
                    family=SANS, anchor="start"))
    out.append(text(500, ly, "colour = the boundary crossed", size=12, fill=DIM,
                    family=SANS, anchor="start"))
    lx = 740
    for colour, lab in ((BLUE, "webview ↔ shell"), (ACCENT, "the wire"),
                        (TEAL, "server ↔ runtime"), (VIOLET, "runtime ↔ child process")):
        out.append(rect(lx, ly - 9, 20, 3, fill=colour, stroke="none", rx=2))
        out.append(text(lx + 28, ly, lab, size=12, fill=DIM, family=SANS, anchor="start"))
        lx += 28 + sans_w(lab, 12) * 1.06 + 30

    out.append("</svg>")
    return "".join(out), height


body, height = build()
write_doc("Sequence.dc.html", body)
print(f"Sequence.dc.html  {W}x{height}")
