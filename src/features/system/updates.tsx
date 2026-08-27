import { useState } from "react";

import { CAN_SELF_INSTALL, useUpdate } from "@/lib/updater";
import { Button, Card, CardTitle } from "@/components/ui";
import { formatRelativeTime } from "@/lib/format";

const LINUX_UPDATE_COMMANDS = [
  ["Arch (AUR)", "paru -Syu hyprlayer-desktop-bin"],
  ["Debian / Ubuntu", "sudo apt install ./Hyprlayer_<version>_amd64.deb"],
  ["Fedora / RHEL", "sudo dnf upgrade ./Hyprlayer-<version>-1.x86_64.rpm"],
];

export function UpdatesCard() {
  const pendingVersion = useUpdate((s) => s.pendingVersion);
  const changelog = useUpdate((s) => s.changelog);
  const installing = useUpdate((s) => s.installing);
  const lastChecked = useUpdate((s) => s.lastChecked);
  const checkNow = useUpdate((s) => s.checkNow);
  const install = useUpdate((s) => s.install);

  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<"available" | "uptodate" | "error" | null>(null);

  const entry = changelog?.find((e) => e.version === pendingVersion) ?? null;
  const busy = installing !== null;

  const onCheck = async () => {
    setChecking(true);
    setResult(null);
    setResult(await checkNow());
    setChecking(false);
  };

  return (
    <Card className="p-0">
      <div className="flex items-center justify-between gap-5 p-4">
        <div className="min-w-0">
          <CardTitle className="mb-0">Updates</CardTitle>
          <p className="mt-[3px] text-xs text-ink-dim">
            {pendingVersion ? (
              <span className="text-warn">{pendingVersion} available</span>
            ) : result === "error" ? (
              <span className="text-warn">Couldn’t check</span>
            ) : lastChecked === null ? (
              "Not checked yet"
            ) : (
              "Up to date"
            )}
            {lastChecked !== null && <> · last checked {formatRelativeTime(lastChecked)}</>}
          </p>
        </div>
        <Button onClick={() => void onCheck()} busy={checking}>
          Check now
        </Button>
      </div>

      {pendingVersion && (
        <div className="mx-4 mb-4 rounded-md border border-accent/40 bg-accent/10 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="text-sm font-medium text-ink">
              Version <span className="font-mono">{pendingVersion}</span> is available
            </span>
            {CAN_SELF_INSTALL && (
              <Button variant="primary" onClick={() => void install()} busy={busy}>
                {installing === "installing" ? "Restarting…" : "Update & restart"}
              </Button>
            )}
          </div>
          {entry?.summary && <p className="mt-2 text-sm text-ink-dim">{entry.summary}</p>}
          {entry?.notes && entry.notes.length > 0 && (
            <ul className="mt-2 list-disc space-y-0.5 pl-5 text-sm text-ink-dim">
              {entry.notes.map((note, i) => (
                <li key={i}>{note}</li>
              ))}
            </ul>
          )}
          {!CAN_SELF_INSTALL && (
            <div className="mt-3 border-t border-edge/60 pt-2">
              <p className="text-sm text-ink-dim">Install it with your package manager:</p>
              <dl className="mt-1.5 space-y-1">
                {LINUX_UPDATE_COMMANDS.map(([distro, command]) => (
                  <div key={distro} className="flex flex-wrap items-baseline gap-x-2">
                    <dt className="text-xs text-ink-dim">{distro}</dt>
                    <dd className="font-mono text-xs text-ink">{command}</dd>
                  </div>
                ))}
              </dl>
            </div>
          )}
        </div>
      )}
    </Card>
  );
}
