# Scenario format

One JSON file per scenario, shared by every client. A client implements each `do` step against its own UI; selectors and APIs never appear here.

| `do` | Params | Records |
|---|---|---|
| `first_interactive` | | `m1.first_interactive_ms`: launcher's wall clock before `exec` to the frame after the sidebar shows the fixture's projects |
| `open_transcript` | | proof: store items and rendered rows |
| `open_terminal` | | attaches the fixture's first terminal |
| `settle` | `secs`, `phase` | pauses; the harness samples memory at the phase |
| `scroll` | `secs`, `pxPerSec` | `m3.scroll.*` frame stats |
| `resize_drag` | `secs`, `minWidth`, `maxWidth`, `height` | `m3.resize.*` |
| `token_stream` | `secs` | `m3.stream.*`; sends `PERF_STREAM`, the fixture's agent streams 400 deltas |
| `terminal` | `name`, `command` | `m4.<name>.wall_ms`; the harness samples CPU at `<name>_begin` and `<name>_end` |
| `echo_latency` | `samples` | `m5.echo_p50_ms`, `m5.echo_p95_ms` |
| `soak` | `secs` | repeats scroll, stream and terminal output; harness samples memory at `soaked` |

A phase handshake: the client writes `phase-<name>` in `HYPRLAYER_PERF_PHASE_DIR` and blocks until the harness writes `ack-<name>`.

`report.json`: `{ client, scenario, errors[], metrics{ "<id>": number }, proof{} }`. `metrics` is flat so `summarize.mjs` needs no schema. The harness adds `m2.*` and the CPU figures.
