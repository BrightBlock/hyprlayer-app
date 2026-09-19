from kit import *

W = 1600
LX = [160, 480, 800, 1120, 1440]
ROW = 34
PHASE_H = 40
MARKERS = {"blue": BLUE, "accent": ACCENT, "teal": TEAL, "violet": VIOLET, "bad": BAD, "dim": DIM}
MARKER_OF = {colour: name for name, colour in MARKERS.items()}


def heading(title, sub):
    return (text(48, 56, title, size=30, fill=INK, family=SANS, weight="600", anchor="start")
            + text(48, 84, sub, size=14.5, fill=DIM, family=SANS, anchor="start"))


def wrap(s, size, width):
    lines, cur = [], ""
    for word in s.split(" "):
        trial = (cur + " " + word).strip()
        if sans_w(trial, size) > width:
            lines.append(cur)
            cur = word
        else:
            cur = trial
    lines.append(cur)
    return lines


def fact_row(y, facts, colour):
    out = []
    fw = (W - 80 - (len(facts) - 1) * 20) / len(facts)
    bodies = [wrap(body, 11.5, fw - 44) for _, body in facts]
    h = max(108, 50 + (max(map(len, bodies)) - 1) * 16 + 22)
    fx = 40
    for (title, _), lines in zip(facts, bodies):
        out.append(rect(fx, y, fw, h, fill=PANEL, stroke=EDGE, rx=10))
        out.append(rect(fx, y + 12, 3, h - 24, fill=colour, stroke="none", rx=2))
        out.append(text(fx + 22, y + 28, title, size=13, fill=INK, family=SANS,
                        weight="600", anchor="start"))
        for i, ln in enumerate(lines):
            out.append(text(fx + 22, y + 50 + i * 16, ln, size=11.5, fill=DIM,
                            family=SANS, anchor="start"))
        fx += fw + 20
    return "".join(out), h


SEQ_PARTS = [
    ("Webview", "React 19 · ChangesPanel", BLUE),
    ("HyprlayerRuntime", "hyprlayer-server · claude::memory", TEAL),
    ("Memory directory", "~/.claude/projects/<key>/memory", ACCENT),
    ("Claude session", "Claude Code's own auto-memory", VIOLET),
    ("Codex session", "codex app-server", VIOLET),
]

SEQ = [
    ("phase", "Claude, unchanged", "the key comes from the repository root, so every worktree of a project shares one directory"),
    ("call", 3, 2, "read MEMORY.md at session start", ACCENT),
    ("note", 820, "at most 200 lines or 25,000 UTF-16 units, and the same file for every Claude tab of the project"),
    ("call", 3, 2, "write a topic file, add a line to the index", ACCENT),
    ("note", 820, "Claude decides what is worth keeping; topic files load on demand"),

    ("phase", "A codex tab launches", "only start opens a new thread; resume, restart and rewind send thread/resume"),
    ("call", 1, 2, "resolve( project )  +  key check", TEAL),
    ("call", 1, 2, "read_index( dir )", TEAL),
    ("ret", 2, 1, "the index, bounded as Claude bounds it", TEAL),
    ("note", 180, "withheld on a key mismatch, and when ~/.codex/config.toml sets developer_instructions, which a thread value would replace"),
    ("call", 1, 4, "thread/start  { developerInstructions }", VIOLET),
    ("note", 500, "the preamble names the directory, calls it read-only, and ranks the notes below the user and the repository"),
    ("note", 500, "thread/resume carries it too, but codex 0.154.0 ignores it there: a resumed thread keeps the index it started with"),
    ("call", 4, 2, "read a topic file when it bears on the task", ACCENT),
    ("note", 500, "the sandbox keeps codex out of the directory except under Bypass permissions, where only the preamble does"),

    ("phase", "The memory view", "opened from the lightbulb in the sidebar panel's header, and fetched only while it is open"),
    ("call", 0, 1, "listProjectMemory( workstream_id )", BLUE),
    ("call", 1, 2, "resolve  +  key check  +  list *.md", TEAL),
    ("note", 500, "the key check: the workstream's latest Claude session must have its transcript under project_key( cwd )"),
    ("ret", 2, 1, "name · path · size · mtime", TEAL),
    ("ret", 1, 0, "ProjectMemory { dir, keyCheck, files }", BLUE),
    ("note", 180, "on a mismatch: no files, a Memory unavailable note, and a warn-toned ! badge"),
    ("frame", "loop", "turnDone or exited in the workstream · worktreeChanged · window focus · the refresh button"),
    ("call", 0, 1, "refetch  ['workstream', id, 'memory']", BLUE),
    ("endframe",),
    ("call", 0, 1, "getDoc( path )  —  the existing document reader", BLUE),
    ("ret", 1, 0, "DocContent { meta, content, mtimeMs }", BLUE),

    ("phase", "An edit", "the one new write path; it overwrites an existing file and cannot leave the directory"),
    ("call", 0, 1, "saveProjectMemoryFile( id, name, content, baseMtimeMs )", BLUE),
    ("note", 500, "a bare *.md name with no separator and no leading dot, naming an existing regular file, never a symlink"),
    ("call", 1, 2, "ensure_not_stale( base_mtime_ms )", TEAL),
    ("call", 1, 2, "create_new( .<name>.<uuid>.tmp )  →  rename", TEAL),
    ("note", 500, "O_EXCL refuses a link planted at the temp name, so a save never writes through one"),
    ("ret", 1, 0, "Ok", BLUE),
    ("note", 180, "the webview invalidates the document and the listing"),
    ("frame", "opt", "the file changed on disk after editing began"),
    ("ret", 1, 0, "doc_conflict", BAD),
    ("note", 180, "Reload from disk re-seeds the draft from disk, so one click cannot overwrite an agent's newer write"),
    ("endframe",),
]


def sequence():
    head_top, head_h = 112, 66
    y = head_top + head_h + 30
    rows, frames, open_frame = [], [], None
    for op in SEQ:
        kind = op[0]
        if kind == "phase":
            y += 20
            rows.append(("phase", y, op[1], op[2]))
            y += PHASE_H
        elif kind in ("call", "ret"):
            rows.append((kind, y) + op[1:])
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
            frames.append(open_frame + (y,))
            y += 12

    bottom = y + 26
    height = bottom + 66
    out = [svg_open(W, height), defs(MARKERS),
           heading("Project memory, in sequence",
                   "Claude Code already reads and writes it. hyprlayer adds a view, a contained edit, and a reader for codex.")]

    for x in LX:
        out.append(line(x, head_top + head_h, x, bottom, stroke=EDGE, sw=1, dash="3 6"))

    for lbl, sub, y0, y1 in frames:
        out.append(rect(108, y0, 1452, y1 - y0, fill="none", stroke=EDGE, rx=8, dash="6 6"))
        tab_w = sans_w(lbl, 11) * 1.1 + 22
        out.append(rect(108, y0, tab_w, 22, fill=PANEL2, stroke=EDGE, rx=6))
        out.append(text(108 + tab_w / 2, y0 + 15, lbl, size=11, fill=DIM, family=SANS,
                        weight="600", spacing="0.06em"))
        out.append(text(108 + tab_w + 12, y0 + 15, sub, size=12, fill=DIM, family=SANS,
                        anchor="start"))

    for row in rows:
        if row[0] == "phase":
            _, ry, name, sub = row
            out.append(rect(40, ry - 2, W - 80, PHASE_H - 8, fill=PANEL, stroke="none", rx=8))
            out.append(rect(40, ry - 2, 3, PHASE_H - 8, fill=ACCENT, stroke="none", rx=2))
            out.append(text(62, ry + 19, name, size=13.5, fill=INK, family=SANS, weight="600",
                            anchor="start"))
            out.append(text(62 + sans_w(name, 13.5) * 1.08 + 18, ry + 19, sub, size=12.5,
                            fill=DIM, family=SANS, anchor="start"))
        elif row[0] == "note":
            _, ry, x0, s = row
            out.append(line(x0, ry + 6, x0, ry + 20, stroke=ACCENT, sw=2))
            out.append(text(x0 + 12, ry + 18, s, size=12, fill=DIM, family=SANS, anchor="start"))
        else:
            kind, ry, a, b, s, colour = row
            ly = ry + 24
            back = kind == "ret"
            x1, x2 = LX[a], LX[b]
            out.append(line(x1, ly, x2, ly, stroke=colour, sw=1.6, dash="6 5" if back else None,
                            marker=MARKER_OF[colour]))
            out.append(label_on_line((x1 + x2) / 2, ly - 8, s, size=12.5,
                                     fill=DIM if back else INK))
            if colour == BAD:
                out.append(line(x2 - 7, ly - 7, x2 + 7, ly + 7, stroke=BAD, sw=2.2))
                out.append(line(x2 - 7, ly + 7, x2 + 7, ly - 7, stroke=BAD, sw=2.2))

    for x, (name, sub, colour) in zip(LX, SEQ_PARTS):
        out.append(rect(x - 125, head_top, 250, head_h, fill=PANEL2, stroke=EDGE, rx=10))
        out.append(rect(x - 125, head_top, 250, 3, fill=colour, stroke="none", rx=2))
        out.append(text(x, head_top + 28, name, size=14.5, fill=INK, family=SANS, weight="600"))
        out.append(text(x, head_top + 48, sub, size=11.5, fill=DIM, family=SANS))
        out.append(rect(x - 55, bottom, 110, 12, fill=PANEL2, stroke=EDGE, rx=4))

    ly = height - 30
    out.append(text(48, ly, "solid = a call", size=12, fill=DIM, family=SANS, anchor="start"))
    out.append(text(190, ly, "dashed = a return", size=12, fill=DIM, family=SANS, anchor="start"))
    out.append(text(350, ly, "colour = the boundary crossed", size=12, fill=DIM, family=SANS,
                    anchor="start"))
    lx = 590
    for colour, lab in ((BLUE, "webview ↔ runtime"), (TEAL, "runtime ↔ memory"),
                        (VIOLET, "runtime ↔ codex"), (ACCENT, "agent ↔ memory"), (BAD, "refused")):
        out.append(rect(lx, ly - 9, 20, 3, fill=colour, stroke="none", rx=2))
        out.append(text(lx + 28, ly, lab, size=12, fill=DIM, family=SANS, anchor="start"))
        lx += 28 + sans_w(lab, 12) * 1.06 + 30

    out.append("</svg>")
    return "".join(out), height


def card(x, y, w, title, lines, colour):
    pad = 14
    h = pad + 18 + len(lines) * 17 + pad - 2
    s = [rect(x, y, w, h, fill=PANEL2, stroke=EDGE, rx=9),
         rect(x, y + 10, 3, h - 20, fill=colour, stroke="none", rx=2),
         text(x + pad + 8, y + pad + 12, title, size=13.5, fill=INK, family=SANS, weight="600",
              anchor="start")]
    for i, ln in enumerate(lines):
        s.append(text(x + pad + 8, y + pad + 32 + i * 17, ln, size=11.5, fill=DIM, family=MONO,
                      anchor="start"))
    return "".join(s), h


def stack(x, y, w, cards):
    out = []
    for title, lines, colour in cards:
        s, h = card(x, y, w, title, lines, colour)
        out.append(s)
        y += h + 11
    return "".join(out), y - 11


def panel(x, y, w, h, title, sub, colour):
    return (rect(x, y, w, h, fill=PANEL, stroke=EDGE, rx=14)
            + rect(x, y, w, 3, fill=colour, stroke="none", rx=2)
            + text(x + 24, y + 36, title, size=17, fill=INK, family=SANS, weight="600",
                   anchor="start")
            + text(x + 24, y + 57, sub, size=12, fill=DIM, family=SANS, anchor="start"))


def chips(x0, y0, limit, names):
    out, x, y = [], x0, y0
    for name in names:
        cw = mono_w(name, 11.5) + 20
        if x + cw > limit:
            x, y = x0, y + 30
        out.append(rect(x, y, cw, 23, fill=SURFACE, stroke=EDGE, rx=6))
        out.append(text(x + cw / 2, y + 16, name, size=11.5, fill=TEAL, family=MONO))
        x += cw + 9
    return "".join(out), y + 23


def architecture():
    TOP = 118
    CX, CW = 40, 440
    SX, SW = 524, 560
    MX, MW = 1128, 432
    body = []

    client, c_end = stack(CX + 22, TOP + 78, CW - 44, [
        ("The memory view", ["the lightbulb in the sidebar panel's header",
                             "MEMORY.md first, then topic files",
                             "fetched only while it is open"], BLUE),
        ("DocPreviewTab", ["reads a file through the existing getDoc",
                           "a pencil on files the listing holds",
                           "drafts survive a tab switch"], BLUE),
        ("Two new commands", ["listProjectMemory(id)",
                              "saveProjectMemoryFile(id, name, …)"], BLUE),
        ("Refresh", ["turnDone · exited · worktreeChanged",
                     "window focus · the refresh button",
                     "through ['workstream', id, 'memory']"], BLUE),
    ])

    hx, hy, hw = SX + 22, TOP + 78, SW - 44
    chip_svg, chip_end = chips(hx + 22, hy + 50, hx + hw - 22,
                               ["project_key()", "resolve()", "bounded_index()", "read_index()",
                                "list_files()", "is_memory_file_name()", "developer_instructions()"])
    hh = chip_end - hy + 22
    server = [rect(hx, hy, hw, hh, fill=PANEL2, stroke=TEAL, rx=9),
              rect(hx, hy, hw, hh, fill=TEAL, stroke="none", rx=9, opacity=0.07),
              text(hx + 22, hy + 26, "claude::memory", size=15, fill=INK, family=SANS,
                   weight="600", anchor="start"),
              text(hx + 22 + sans_w("claude::memory", 15) * 1.08 + 14, hy + 26,
                   "free functions over explicit paths — no runtime import", size=11.5,
                   fill=DIM, family=SANS, anchor="start"),
              chip_svg]
    s_stack, s_end = stack(hx, hy + hh + 11, hw, [
        ("rpc.rs · dispatch.rs", ["ListProjectMemory · SaveProjectMemoryFile",
                                  "one arm each, straight to the facade"], TEAL),
        ("WorkstreamService", ["project_memory(id)  →  ProjectMemory { dir, keyCheck, files }",
                               "save_project_memory_file(id, name, content, base_mtime_ms)"], TEAL),
        ("SessionService — the codex arm", ["developerInstructions on thread/start and thread/resume",
                                            "no key mismatch ·",
                                            "no developer_instructions in config.toml"], VIOLET),
    ])
    server.append(s_stack)

    memory, m_end = stack(MX + 22, TOP + 78, MW - 44, [
        ("MEMORY.md", ["the index — Claude loads the first",
                       "200 lines or 25,000 UTF-16 units"], ACCENT),
        ("Topic files", ["one note per *.md file,", "read on demand"], ACCENT),
        ("Claude session", ["reads and writes it natively;",
                            "hyprlayer changes nothing here"], VIOLET),
        ("Codex session", ["gets the index at thread start,",
                           "reads topic files, asked not to write"], VIOLET),
        ("You", ["edit an existing file from", "the document preview"], BLUE),
    ])

    ph = max(c_end, s_end, m_end) + 22 - TOP
    body.append(panel(CX, TOP, CW, ph, "Client machine",
                      "Hyprlayer.app — sends a workstream id and a bare file name", BLUE))
    body.append(client)
    body.append(panel(SX, TOP, SW, ph, "Server machine",
                      "hyprlayer-server — the runtime, on the machine the memory lives on", TEAL))
    body.extend(server)
    body.append(panel(MX, TOP, MW, ph, "The memory directory",
                      "Claude Code's own — one per repository", ACCENT))
    body.append(memory)

    for x1, x2, colour in ((CX + CW + 6, SX - 10, BLUE), (SX + SW + 6, MX - 10, TEAL)):
        body.append(line(x1, TOP + 270, x2, TOP + 270, stroke=colour, sw=2,
                         marker=MARKER_OF[colour]))
        body.append(line(x2, TOP + 310, x1, TOP + 310, stroke=colour, sw=2,
                         marker=MARKER_OF[colour], dash="6 5"))

    by, bh = TOP + ph + 34, 176
    bw = SX + SW - CX
    body.append(rect(CX, by, bw, bh, fill=PANEL, stroke=EDGE, rx=14))
    body.append(text(CX + 24, by + 34, "One listing, through every layer", size=17, fill=INK,
                     family=SANS, weight="600", anchor="start"))
    body.append(text(CX + 24, by + 55, "the save takes the same route, ending in save_project_memory_file",
                     size=12, fill=DIM, family=SANS, anchor="start"))
    chain = [("useProjectMemory", "src/lib/queries.ts", BLUE),
             ("list_project_memory", "src-tauri command", BLUE),
             ("ListProjectMemory", "hyprlayer-server rpc", TEAL),
             ("project_memory", "WorkstreamService", TEAL),
             ("resolve · list_files", "claude::memory", TEAL)]
    cw = (bw - 52 - 4 * 30) / 5
    cx = CX + 26
    for i, (name, sub, colour) in enumerate(chain):
        body.append(rect(cx, by + 78, cw, 56, fill=PANEL2, stroke=EDGE, rx=9))
        body.append(rect(cx, by + 78, cw, 3, fill=colour, stroke="none", rx=2))
        body.append(text(cx + cw / 2, by + 102, name, size=12.5, fill=INK, family=MONO,
                         weight="500"))
        body.append(text(cx + cw / 2, by + 121, sub, size=11, fill=DIM, family=SANS))
        if i < 4:
            body.append(line(cx + cw + 6, by + 106, cx + cw + 24, by + 106, stroke=DIM, sw=1.6,
                             marker="dim"))
        cx += cw + 30

    body.append(rect(MX, by, MW, bh, fill=PANEL, stroke=EDGE, rx=14))
    body.append(rect(MX, by, MW, 3, fill=VIOLET, stroke="none", rx=2))
    body.append(text(MX + 24, by + 34, "What else the runtime reads", size=17, fill=INK,
                     family=SANS, weight="600", anchor="start"))
    body.append(text(MX + 24, by + 55, "and it writes only inside the memory directory", size=12,
                     fill=DIM, family=SANS, anchor="start"))
    for i, (name, desc) in enumerate([("settings.json", "autoMemoryDirectory, managed then user"),
                                      ("hyprlayer.db", "the latest Claude session"),
                                      ("<session>.jsonl", "its transcript, for the key check"),
                                      ("config.toml", "codex's own developer_instructions")]):
        ry = by + 86 + i * 22
        body.append(text(MX + 24, ry, name, size=11.5, fill=INK, family=MONO, anchor="start"))
        body.append(text(MX + 154, ry, desc, size=11, fill=DIM, family=SANS, anchor="start"))

    ny = by + bh + 26
    facts, fh = fact_row(ny, [
        ("The key is Claude Code's convention, not ours",
         "The encoding, the autoMemoryDirectory precedence and the index bounds are ported from "
         "Claude Code and were checked against 2.1.252, .258, .270, .273 and .274. There is no "
         "version gate: if a release moves the directory, the key check reports a mismatch and "
         "the view and codex both get nothing."),
        ("The sandbox, not the preamble, stops a codex write",
         "Under Default and Accept-edits the directory is outside codex's writable roots, and Plan "
         "runs read-only. Bypass permissions runs danger-full-access, where the preamble's request "
         "is the only thing between codex and a write."),
        ("The one write path is contained",
         "A bare *.md name, an existing regular file, no symlinked target, a stale-write guard, and "
         "an O_EXCL temp file. The Library's own save still renames through a fixed temp name "
         "(library.rs, gitops.rs); that fix is separate."),
    ], ACCENT)
    body.append(facts)

    height = ny + fh + 50
    return svg_open(W, height) + defs(MARKERS) + heading(
        "One directory, three readers",
        "Claude Code owns the memory directory. hyprlayer finds it, shows it, lets you edit a file in it, and hands its index to codex tabs.",
    ) + "".join(body) + "</svg>", height


COVERAGE = [
    ("Claude chat session", "start · resume · restart · rewind", TEAL, "the index, loaded by Claude",
     "topic files on demand", True, "Claude Code's own auto-memory, unchanged"),
    ("Codex chat, new thread", "start", TEAL, "the index, as developerInstructions",
     "topic files, read-only", False, "no key mismatch, no config.toml value"),
    ("Codex chat, resumed", "resume · restart · rewind", ACCENT, "the index from its thread start",
     "topic files, read-only", False, "codex 0.154.0 ignores it on thread/resume"),
    ("Claude Code terminal", "the multiplexer's claude-code engine", TEAL, "the index, loaded by Claude",
     "topic files on demand", True, "plain claude; the view catches up on focus"),
    ("Other agent terminals", "codex · gemini · antigravity · custom", BAD, "nothing",
     "—", False, "plain CLIs, not launched through launch_driver"),
    ("OpenCode session", "one shared opencode serve", BAD, "nothing",
     "—", False, "not wired for delivery or editing"),
    ("Title generation", "claude · codex · opencode", BAD, "nothing",
     "—", False, "a one-shot outside the project; codex passes None"),
]


def status_box(x, y, colour):
    glyph = "×" if colour == BAD else "✓"
    return (rect(x, y, 18, 18, fill=SURFACE, stroke=colour, rx=5)
            + text(x + 9, y + 13, glyph, size=12, fill=colour, family=SANS, weight="600"))


def coverage():
    top, rh = 118, 64
    out = [heading("What each launch gets",
                   "Memory reaches a session only through its harness's own startup path. Four launches carry it; three do not.")]

    lx = 880
    for colour, lab in ((TEAL, "at launch"), (ACCENT, "as of the thread's start"), (BAD, "none")):
        out.append(status_box(lx, 45, colour))
        out.append(text(lx + 28, 58, lab, size=12, fill=DIM, family=SANS, anchor="start"))
        lx += 28 + sans_w(lab, 12) * 1.06 + 30

    th = 60 + rh * len(COVERAGE) + 16
    out.append(rect(40, top, W - 80, th, fill=PANEL, stroke=EDGE, rx=14))
    for x, label in ((64, "Launch"), (420, "At start"), (740, "Mid-session"), (1000, "Writes"),
                     (1100, "Why")):
        out.append(text(x, top + 38, label.upper(), size=11, fill=DIM, family=SANS, weight="600",
                        anchor="start", spacing="0.06em"))
    for i, (name, sub, start, start_label, mid, writes, why) in enumerate(COVERAGE):
        y = top + 60 + i * rh
        out.append(line(56, y, W - 56, y, stroke=EDGE, sw=1))
        out.append(text(64, y + 28, name, size=13.5, fill=INK, family=SANS, weight="600",
                        anchor="start"))
        out.append(text(64, y + 47, sub, size=11, fill=DIM, family=SANS, anchor="start"))
        out.append(status_box(420, y + 17, start))
        out.append(text(448, y + 30, start_label, size=12, fill=DIM if start == BAD else INK,
                        family=MONO, anchor="start"))
        out.append(text(740, y + 30, mid, size=12, fill=DIM, family=MONO, anchor="start"))
        out.append(status_box(1000, y + 17, TEAL if writes else BAD))
        out.append(text(1100, y + 30, why, size=12, fill=DIM, family=SANS, anchor="start"))

    ny = top + th + 34
    facts, fh = fact_row(ny, [
        ("When the view refreshes",
         "On turnDone or exited of a session in the workstream and on worktreeChanged, because "
         "those invalidate the ['workstream', id] prefix the listing sits under; on window focus "
         "once the listing is 5 s old; and from the refresh button. It is fetched only while the "
         "view is open or a document preview is showing."),
        ("Where it can still be stale",
         "Another workstream of the same project writes memory while you watch this one; a Claude "
         "Code terminal writes, and emits no session events; or Claude's optional post-turn "
         "extraction writes after turnDone. Focus, the refresh button or reopening the view picks "
         "up all three."),
    ], VIOLET)
    out.append(facts)

    height = ny + fh + 50
    return svg_open(W, height) + defs(MARKERS) + "".join(out) + "</svg>", height


for filename, (body, height) in (("MemorySequence.dc.html", sequence()),
                                 ("MemoryMain.dc.html", architecture()),
                                 ("MemoryCoverage.dc.html", coverage())):
    write_doc(filename, body)
    print(f"{filename}  {W}x{height}")
