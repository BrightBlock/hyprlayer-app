import { useState } from "react";

import type { Host, ProvisionStep } from "@/lib/bindings";
import type { HostProbeResult } from "@/lib/domain/hosts";
import { errorMessage } from "@/lib/ipc";
import { useTestHostConnection } from "@/lib/queries";
import { shortFailure } from "./failures";
import { retryConnection } from "./reconnect";
import { NO_SERVER, runProvision } from "./serverInstall";

// `code` is kept because a missing server is a failure a row can act on, not only report.
export type HostCheck =
  | { ok: true; at: number }
  | { ok: false; code?: string; short: string; detail?: string; at: number };

function fromProbe(probe: HostProbeResult): HostCheck {
  return probe.ok
    ? { ok: true, at: probe.at }
    : { ok: false, code: probe.code, short: shortFailure(probe.code), at: probe.at };
}

// A newer shared probe (made from another surface) outranks this surface's own.
function newest(
  shared: Record<string, HostProbeResult>,
  own: Record<string, HostCheck>,
): Record<string, HostCheck> {
  const merged: Record<string, HostCheck> = {};
  for (const [destination, probe] of Object.entries(shared)) merged[destination] = fromProbe(probe);
  for (const [destination, check] of Object.entries(own)) {
    const theirs = merged[destination];
    if (!theirs || check.at >= theirs.at) merged[destination] = check;
  }
  return merged;
}

/** Tests and installs hosts; every answered test also lands in the session's shared probes. */
export function useHostChecks(active: string | null, shared?: Record<string, HostProbeResult>) {
  const test = useTestHostConnection();
  const [own, setOwn] = useState<Record<string, HostCheck>>({});
  const [testing, setTesting] = useState<string | null>(null);
  const [testingAll, setTestingAll] = useState(false);
  // One at a time, like `testing`: this pushes 16MB over ssh, and a second
  // concurrent push is a way to saturate a link, not a feature.
  const [installing, setInstalling] = useState<{ destination: string; step: ProvisionStep } | null>(
    null,
  );

  const record = (destination: string, check: HostCheck) =>
    setOwn((prev) => ({ ...prev, [destination]: check }));

  async function runTest(host: Host) {
    setTesting(host.destination);
    try {
      const probe = await test.mutateAsync(host);
      if (probe.ok) {
        record(host.destination, { ok: true, at: Date.now() });
        // The connection surface above is still reporting the failure this host
        // just disproved. Reconnecting is what makes the two agree — and it is
        // the same work a relaunch would do, without the relaunch.
        if (host.destination === active) void retryConnection();
      } else {
        record(host.destination, {
          ok: false,
          code: probe.error?.code,
          short: shortFailure(probe.error?.code),
          detail: errorMessage(probe.error ?? "Could not connect."),
          at: Date.now(),
        });
      }
    } catch (e) {
      record(host.destination, { ok: false, short: "Failed", detail: errorMessage(e), at: Date.now() });
    } finally {
      setTesting(null);
    }
  }

  // Provisioning a host it has not switched to yet is the point: the probe above
  // already knows the server is missing, and learning that only after a restart
  // into a blocked window cost a second one to get out of.
  async function runInstall(host: Host) {
    setInstalling({ destination: host.destination, step: "probe" });
    try {
      await runProvision(host, (step) =>
        setInstalling((cur) => (cur?.destination === host.destination ? { ...cur, step } : cur)),
      );
      setInstalling(null);
      // The row's own probe is what says it worked; nothing here claims it.
      await runTest(host);
    } catch (e) {
      setInstalling(null);
      // Still the code that offers the action: a failed push leaves the server
      // as missing as it was, so retrying stays one click rather than two.
      record(host.destination, {
        ok: false,
        code: NO_SERVER,
        short: "Install failed",
        detail: errorMessage(e),
        at: Date.now(),
      });
    }
  }

  async function testAll(hosts: Host[]) {
    setTestingAll(true);
    try {
      for (const host of hosts) await runTest(host);
    } finally {
      setTestingAll(false);
    }
  }

  return {
    checks: shared ? newest(shared, own) : own,
    testing,
    testingAll,
    installing,
    runTest,
    runInstall,
    testAll,
  };
}
