// Perf harness probe (scripts/perf). Reached only behind VITE_PERF_PROBE; a normal build drops it.
import { invoke } from "@tauri-apps/api/core";

import { writeTerminal } from "./terminal/terminalInput";

/** What the probe needs from the app; `src/perfWiring.ts` implements it where the layering allows. */
export interface PerfHost {
  projectsRendered(): Promise<void>;
  openTranscript(): Promise<void>;
  transcriptProof(): { storeItems: number; domRows: number };
  scroller(): HTMLElement | null;
  sendMessage(text: string): Promise<void>;
  transcriptShows(text: string): boolean;
  openTerminal(id: string): Promise<void>;
  terminalGrid(): { cols: number; rows: number } | null;
}

type Step = { do: string; [key: string]: unknown };
type Config = {
  scenario: { name: string; steps: Step[] };
  t0Ns: number | null;
  bigFile: string | null;
  terminals: string[];
};

const metrics: Record<string, number> = {};
const proof: Record<string, unknown> = {};
const errors: string[] = [];

const call = <T>(op: string, payload?: unknown) => invoke<T>("perf_probe", { op, payload });
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const nowMs = () => performance.timeOrigin + performance.now();
const round = (n: number) => Math.round(n * 100) / 100;

// The terminal tap: every acked frame's bytes, so a wait can see its marker and an echo.
let framesAcked = 0;
let bytesAcked = 0;
let lastFrameTail = "";
let waiter: { needle: string; resolve: (at: number) => void } | null = null;
const decoder = new TextDecoder("latin1");

/** Called from the terminal feed once xterm has parsed a screen frame. */
export function terminalFrame(bytes: Uint8Array): void {
  framesAcked += 1;
  bytesAcked += bytes.length;
  if (!waiter) return;
  const text = lastFrameTail + decoder.decode(bytes);
  lastFrameTail = text.slice(-128);
  if (text.includes(waiter.needle)) {
    const done = waiter;
    waiter = null;
    done.resolve(nowMs());
  }
}

function waitForFrame(needle: string, timeoutMs: number): Promise<number> {
  lastFrameTail = "";
  return new Promise((resolve, reject) => {
    waiter = { needle, resolve };
    setTimeout(() => {
      if (waiter?.needle === needle) {
        waiter = null;
        reject(new Error(`no frame showed ${needle} within ${timeoutMs} ms`));
      }
    }, timeoutMs);
  });
}

type Frames = { frames: number; p50: number; p95: number; p99: number; max: number; over33Pct: number };

/** Per-frame deltas over `ms`, with `drive` called every frame to move whatever is being measured. */
function recordFrames(ms: number, drive?: (elapsed: number, dt: number) => void): Promise<Frames> {
  return new Promise((resolve) => {
    const deltas: number[] = [];
    let start = 0;
    let last = 0;
    const tick = (t: number) => {
      if (!start) start = last = t;
      const dt = t - last;
      if (t !== start) deltas.push(dt);
      last = t;
      drive?.(t - start, dt);
      if (t - start < ms) return requestAnimationFrame(tick);
      const sorted = [...deltas].sort((a, b) => a - b);
      const q = (p: number) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] ?? 0;
      resolve({
        frames: deltas.length,
        p50: round(q(0.5)),
        p95: round(q(0.95)),
        p99: round(q(0.99)),
        max: round(sorted[sorted.length - 1] ?? 0),
        over33Pct: round((deltas.filter((d) => d > 33.4).length / Math.max(1, deltas.length)) * 100),
      });
    };
    requestAnimationFrame(tick);
  });
}

function record(prefix: string, f: Frames): void {
  metrics[`${prefix}.frames`] = f.frames;
  metrics[`${prefix}.p50_ms`] = f.p50;
  metrics[`${prefix}.p95_ms`] = f.p95;
  metrics[`${prefix}.p99_ms`] = f.p99;
  metrics[`${prefix}.max_ms`] = f.max;
  metrics[`${prefix}.over33_pct`] = f.over33Pct;
}

const num = (step: Step, key: string, fallback: number): number =>
  typeof step[key] === "number" ? (step[key] as number) : fallback;

async function scroll(host: PerfHost, secs: number, pxPerSec: number): Promise<Frames> {
  const el = host.scroller();
  if (!el) throw new Error("no transcript scroller");
  el.scrollTop = el.scrollHeight;
  let dir = -1;
  return recordFrames(secs * 1000, (_, dt) => {
    const next = el.scrollTop + (dir * pxPerSec * dt) / 1000;
    if (next <= 0 || next >= el.scrollHeight - el.clientHeight) dir = -dir;
    el.scrollTop = next;
  });
}

async function resizeDrag(secs: number, minW: number, maxW: number, height: number): Promise<Frames> {
  let pending = false;
  return recordFrames(secs * 1000, (elapsed) => {
    // One resize in flight at a time, like a drag the compositor paces.
    if (pending) return;
    const width = minW + ((maxW - minW) * (1 + Math.sin(elapsed / 700))) / 2;
    pending = true;
    void call("resize", { width: Math.round(width), height }).finally(() => (pending = false));
  });
}

async function tokenStream(host: PerfHost, secs: number): Promise<Frames> {
  const el = host.scroller();
  if (el) el.scrollTop = el.scrollHeight;
  await host.sendMessage("PERF_STREAM");
  const frames = await recordFrames(secs * 1000);
  const deadline = Date.now() + 30_000;
  while (!host.transcriptShows("PERF_STREAM_END") && Date.now() < deadline) await sleep(100);
  if (!host.transcriptShows("PERF_STREAM_END")) errors.push("the token stream never finished");
  return frames;
}

async function terminalRun(id: string, command: string, marker: string): Promise<{ wallMs: number }> {
  const done = waitForFrame(marker, 180_000);
  const started = nowMs();
  writeTerminal(id, `${command}\r`);
  const at = await done;
  return { wallMs: round(at - started) };
}

const quantile = (xs: number[], p: number) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(s.length * p))] ?? 0;
};

async function echoLatency(id: string, samples: number): Promise<void> {
  const times: number[] = [];
  for (let i = 0; i < samples; i++) {
    const shown = waitForFrame("Q", 5_000);
    const started = nowMs();
    writeTerminal(id, "Q");
    try {
      times.push((await shown) - started);
    } catch (e) {
      errors.push(String(e));
    }
    writeTerminal(id, "\x7f");
    await sleep(150);
  }
  metrics["m5.echo_p50_ms"] = round(quantile(times, 0.5));
  metrics["m5.echo_p95_ms"] = round(quantile(times, 0.95));
  proof["m5.samples"] = times.length;
}

async function run(host: PerfHost, config: Config): Promise<void> {
  const terminal = config.terminals[0];
  const phase = (name: string) => call("phase", name);
  const guarded = async (label: string, work: () => Promise<void>) => {
    try {
      await work();
    } catch (e) {
      errors.push(`${label}: ${String(e)}`);
    }
  };

  for (const step of config.scenario.steps) {
    const label = step.do;
    // rAF stalls in a hidden window, so every measured step records what the page saw.
    proof[`visibility.${label}`] = document.visibilityState;
    await guarded(label, async () => {
      switch (step.do) {
        case "first_interactive": {
          await host.projectsRendered();
          const t0 = config.t0Ns ? config.t0Ns / 1e6 : null;
          if (t0) metrics["m1.first_interactive_ms"] = round(nowMs() - t0);
          break;
        }
        case "open_transcript":
          await host.openTranscript();
          break;
        case "open_terminal": {
          if (!terminal) throw new Error("the fixture has no terminal");
          const first = waitForFrame("", 30_000);
          await host.openTerminal(terminal);
          await first;
          break;
        }
        case "settle":
          await sleep(num(step, "secs", 60) * 1000);
          await phase(String(step.phase ?? "settled"));
          break;
        case "scroll":
          record("m3.scroll", await scroll(host, num(step, "secs", 10), num(step, "pxPerSec", 1500)));
          break;
        case "resize_drag":
          record(
            "m3.resize",
            await resizeDrag(num(step, "secs", 10), num(step, "minWidth", 1100), num(step, "maxWidth", 1500), num(step, "height", 900)),
          );
          break;
        case "token_stream":
          record("m3.stream", await tokenStream(host, num(step, "secs", 10)));
          break;
        case "terminal": {
          if (!terminal) throw new Error("the fixture has no terminal");
          const name = String(step.name);
          const file = config.bigFile ?? "";
          const command = String(step.command).replace("$BIGFILE", file);
          const marker = `PERF_${name.toUpperCase()}_END`;
          await phase(`${name}_begin`);
          const before = bytesAcked;
          const result = await terminalRun(terminal, `${command}; printf 'PERF_%s\\n' ${name.toUpperCase()}_END`, marker);
          await phase(`${name}_end`);
          metrics[`m4.${name}.wall_ms`] = result.wallMs;
          proof[`m4.${name}.bytes_acked`] = bytesAcked - before;
          proof[`m4.${name}.marker_seen`] = true;
          break;
        }
        case "echo_latency":
          if (!terminal) throw new Error("the fixture has no terminal");
          await echoLatency(terminal, num(step, "samples", 30));
          break;
        case "soak": {
          if (!terminal) throw new Error("the fixture has no terminal");
          const until = Date.now() + num(step, "secs", 600) * 1000;
          let rounds = 0;
          while (Date.now() < until) {
            await scroll(host, 8, 1500);
            await tokenStream(host, 6);
            await terminalRun(terminal, "seq 1 100000; printf 'PERF_%s\\n' SOAK_END", "PERF_SOAK_END");
            rounds += 1;
          }
          proof["soak.rounds"] = rounds;
          await phase("soaked");
          break;
        }
        default:
          throw new Error(`unknown step ${step.do}`);
      }
    });
  }
  proof["transcript"] = host.transcriptProof();
  proof["terminal.grid"] = host.terminalGrid();
  proof["terminal.frames_acked"] = framesAcked;
  proof["terminal.bytes_acked"] = bytesAcked;
  proof["window"] = { width: window.innerWidth, height: window.innerHeight, visibility: document.visibilityState };
}

export async function startPerfProbe(host: PerfHost): Promise<void> {
  window.addEventListener("error", (e) => errors.push(`error: ${e.message}`));
  window.addEventListener("unhandledrejection", (e) => errors.push(`rejection: ${String(e.reason)}`));
  const raw = await call<{
    scenario: Config["scenario"];
    t0Ns: number | null;
    bigFile: string | null;
    terminals: string[];
  }>("scenario");
  const config: Config = raw;
  await run(host, config);
  await call("report", {
    client: "tauri",
    scenario: config.scenario.name,
    errors,
    metrics,
    proof,
  });
}
