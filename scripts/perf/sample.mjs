#!/usr/bin/env node
// One process-set sample as JSON: PSS or phys footprint, GPU memory, CPU and per-thread CPU.
// Linux: the descendants of --root. macOS: those plus com.apple.WebKit.* pids not in --exclude.
// Usage: sample.mjs --root <pid> [--exclude a,b]

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const arg = (name) => {
  const i = process.argv.indexOf(`--${name}`);
  return i < 0 ? "" : process.argv[i + 1];
};
const root = Number(arg("root"));
const exclude = new Set(arg("exclude").split(",").filter(Boolean).map(Number));
const linux = process.platform === "linux";
const read = (p) => {
  try {
    return readFileSync(p, "utf8");
  } catch {
    return "";
  }
};

function pidsLinux() {
  const ppid = new Map();
  for (const d of readdirSync("/proc").filter((d) => /^\d+$/.test(d))) {
    const stat = read(`/proc/${d}/stat`);
    const m = stat.match(/^\d+ \(.*\) \S (\d+)/s);
    if (m) ppid.set(Number(d), Number(m[1]));
  }
  const out = new Set([root]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const [pid, parent] of ppid) if (out.has(parent) && !out.has(pid)) (out.add(pid), (grew = true));
  }
  return [...out].filter((p) => ppid.has(p));
}

function sampleLinux(pids) {
  const ticks = 100;
  let pssKb = 0;
  let gpuKb = 0;
  let cpuMs = 0;
  const procs = [];
  const threads = [];
  for (const pid of pids) {
    pssKb += Number(read(`/proc/${pid}/smaps_rollup`).match(/^Pss:\s+(\d+)/m)?.[1] ?? 0);
    const seen = new Set();
    for (const fd of (() => { try { return readdirSync(`/proc/${pid}/fdinfo`); } catch { return []; } })()) {
      const info = read(`/proc/${pid}/fdinfo/${fd}`);
      const id = info.match(/^drm-client-id:\s+(\d+)/m)?.[1];
      if (!id || seen.has(id)) continue;
      seen.add(id);
      for (const m of info.matchAll(/^drm-(?:total|resident)-\S+:\s+(\d+) (\w+)/gm)) {
        gpuKb += Number(m[1]) * ({ KiB: 1, MiB: 1024, GiB: 1048576 }[m[2]] ?? 1);
      }
    }
    const comm = read(`/proc/${pid}/comm`).trim();
    let procMs = 0;
    for (const tid of (() => { try { return readdirSync(`/proc/${pid}/task`); } catch { return []; } })()) {
      const stat = read(`/proc/${pid}/task/${tid}/stat`);
      const f = stat.slice(stat.lastIndexOf(")") + 2).split(" ");
      const ms = ((Number(f[11]) + Number(f[12])) * 1000) / ticks;
      procMs += ms;
      threads.push({ pid, tid: Number(tid), name: read(`/proc/${pid}/task/${tid}/comm`).trim(), cpuMs: ms });
    }
    cpuMs += procMs;
    procs.push({ pid, comm, cpuMs: procMs });
  }
  // NVIDIA's fdinfo carries no drm-memory keys; its per-process graphics memory comes from nvidia-smi.
  let gpuSource = "fdinfo";
  if (!gpuKb) {
    try {
      const q = execFileSync("nvidia-smi", ["-q", "-d", "PIDS"], { encoding: "utf8" });
      gpuSource = "nvidia-smi";
      for (const m of q.matchAll(/Process ID\s+:\s+(\d+)[\s\S]*?Used GPU Memory\s+:\s+(\d+) MiB/g)) {
        if (pids.includes(Number(m[1]))) gpuKb += Number(m[2]) * 1024;
      }
    } catch {
      gpuSource = "none";
    }
  }
  return { pssKb, gpuKb, gpuSource, cpuMs, procs, threads };
}

function sampleMac() {
  const ps = execFileSync("ps", ["-axo", "pid=,ppid=,comm="], { encoding: "utf8" })
    .split("\n")
    .map((l) => l.trim().match(/^(\d+)\s+(\d+)\s+(.*)$/))
    .filter(Boolean)
    .map((m) => ({ pid: Number(m[1]), ppid: Number(m[2]), comm: m[3] }));
  const set = new Set([root]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const p of ps) if (set.has(p.ppid) && !set.has(p.pid)) (set.add(p.pid), (grew = true));
  }
  for (const p of ps) if (p.comm.includes("com.apple.WebKit.") && !exclude.has(p.pid)) set.add(p.pid);
  const procs = [];
  let footprintKb = 0;
  let cpuMs = 0;
  for (const pid of set) {
    let kb = 0;
    try {
      const file = join(tmpdir(), `perf-footprint-${process.pid}.json`);
      execFileSync("footprint", ["-p", String(pid), "--json", file], { stdio: "ignore" });
      kb = Math.round(Number(JSON.parse(readFileSync(file, "utf8")).processes?.[0]?.footprint ?? 0) / 1024);
      rmSync(file, { force: true });
    } catch {}
    let ms = 0;
    try {
      const t = execFileSync("ps", ["-o", "time=", "-p", String(pid)], { encoding: "utf8" }).trim();
      const [m, s] = t.split(":");
      ms = (Number(m) * 60 + Number(s)) * 1000;
    } catch {}
    footprintKb += kb;
    cpuMs += ms;
    procs.push({ pid, comm: ps.find((p) => p.pid === pid)?.comm ?? "", footprintKb: kb, cpuMs: ms });
  }
  return { pssKb: footprintKb, gpuKb: 0, cpuMs, procs, threads: [] };
}

console.log(JSON.stringify(linux ? sampleLinux(pidsLinux()) : sampleMac()));
