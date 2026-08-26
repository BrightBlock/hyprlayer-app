import { open } from "@tauri-apps/plugin-dialog";

import { Badge, Button, Card, Spinner } from "@/components/ui";
import { useCopyFeedback } from "@/lib/clipboard";
import { errorMessage } from "@/lib/ipc";
import {
  useBootstrap,
  useCliInstallMethods,
  useInstallHyprlayerCli,
  useRedetectBinaries,
  useSetBinaryOverride,
} from "@/lib/queries";
import type { CliInstallMethod } from "@/lib/bindings";
import { Step } from "./StepShell";

const STEP_NUMBER = 2;
const STEP_TOTAL = 4;
const TITLE = "Install the Hyprlayer CLI";

export function CliInstallStep({ onNext }: { onNext: () => void }) {
  const bootstrap = useBootstrap();
  const hl = bootstrap.data?.binaries.find((b) => b.kind === "hyprlayer");
  const hlOk = hl?.state.state === "ok";

  if (bootstrap.isLoading) {
    return (
      <Step number={STEP_NUMBER} total={STEP_TOTAL} title={TITLE}>
        <Card className="flex items-center justify-center py-12">
          <Spinner />
        </Card>
      </Step>
    );
  }

  return (
    <Step
      number={STEP_NUMBER}
      total={STEP_TOTAL}
      title={TITLE}
      footer={
        <Button variant="primary" onClick={onNext} disabled={!hlOk}>
          Next
        </Button>
      }
    >
      {hlOk ? (
        <Card className="space-y-3">
          <div className="flex items-center gap-2">
            <span className="text-lg">✓</span>
            <p className="text-sm font-medium text-ink">Hyprlayer CLI is installed</p>
          </div>
          <p className="text-sm text-ink-dim">
            Found {hl?.version ? `v${hl.version}` : "the CLI"}
            {hl?.path ? (
              <>
                {" "}
                at <span className="break-all font-mono text-xs">{hl.path}</span>
              </>
            ) : null}
            .
          </p>
        </Card>
      ) : (
        <InstallPicker />
      )}
    </Step>
  );
}

function InstallPicker() {
  const bootstrap = useBootstrap();
  const methods = useCliInstallMethods();
  const install = useInstallHyprlayerCli();
  const redetect = useRedetectBinaries();
  const override = useSetBinaryOverride();

  const all = methods.data ?? [];
  const available = all.filter((m) => m.available);
  const installingLabel = install.isPending
    ? (all.find((m) => m.id === install.variables)?.label ?? null)
    : null;
  const installedButMissing = install.isSuccess && !bootstrap.isFetching;

  async function locate() {
    const picked = await open({ title: "Locate hyprlayer" });
    if (typeof picked === "string") {
      override.mutate({ kind: "hyprlayer", path: picked });
    }
  }

  return (
    <div className="space-y-3">
      <Card>
        <p className="text-sm text-ink-dim">
          Hyprlayer's command-line tool powers your config, thoughts storage, and the AI setup in
          the next steps. Install it to continue.
        </p>
      </Card>

      {installedButMissing && (
        <Card className="space-y-2 border-warn/40">
          <p className="text-sm text-ink">
            The installer finished, but <span className="font-mono text-xs">hyprlayer</span> isn't
            detected yet.
          </p>
          <div className="flex items-center gap-2">
            <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
              Re-detect
            </Button>
            <span className="text-xs text-ink-dim">
              Still missing? Restart the app, or use “Locate…” below.
            </span>
          </div>
        </Card>
      )}

      <Card className="space-y-3">
        {methods.isLoading ? (
          <div className="flex items-center justify-center py-6">
            <Spinner />
          </div>
        ) : install.isPending ? (
          <div className="flex items-center gap-2 py-2 text-sm text-ink-dim">
            <Spinner size={14} /> Installing{installingLabel ? ` with ${installingLabel}` : ""}…
          </div>
        ) : available.length > 0 ? (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="text-sm font-medium text-ink">Install with a package manager</p>
            <div className="flex flex-wrap justify-end gap-2">
              {available.map((m) => (
                <Button key={m.id} variant="primary" onClick={() => install.mutate(m.id)}>
                  Install with {m.label}
                </Button>
              ))}
            </div>
          </div>
        ) : (
          <p className="text-sm text-ink-dim">
            No supported package manager was found on your system. Install one of the options below,
            or point Hyprlayer at an existing install.
          </p>
        )}
        {install.isError && (
          <p className="selectable break-words text-xs text-bad">
            Couldn't install: {errorMessage(install.error)}
          </p>
        )}
      </Card>

      <Card className="space-y-3">
        <p className="text-sm font-medium text-ink">Or set it up yourself</p>
        {all.length > 0 && (
          <div className="space-y-2">
            {all.map((m) => (
              <CommandRow key={m.id} method={m} />
            ))}
          </div>
        )}
        <div className="flex items-center gap-2 pt-1">
          <Button onClick={() => void locate()} busy={override.isPending}>
            Locate existing install…
          </Button>
          <span className="text-xs text-ink-dim">
            Already have <span className="font-mono">hyprlayer</span>? Point us at it.
          </span>
        </div>
        {override.isError && (
          <p className="selectable break-words text-xs text-bad">
            Couldn't save the path: {errorMessage(override.error)}
          </p>
        )}
      </Card>
    </div>
  );
}

function CommandRow({ method }: { method: CliInstallMethod }) {
  const { copied, trigger } = useCopyFeedback();

  return (
    <div className="flex items-center justify-between gap-3 rounded-md border border-edge bg-panel-2 p-2">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-ink">{method.label}</span>
          {!method.available && <Badge tone="warn">not installed</Badge>}
        </div>
        <p className="selectable mt-0.5 truncate font-mono text-xs text-ink-dim" title={method.command}>
          {method.command}
        </p>
      </div>
      <Button onClick={() => trigger(method.command)} className="shrink-0 text-xs">
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}
