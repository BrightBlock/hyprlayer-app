import { useRef, useState } from "react";

import { Icon } from "@/components/Icon";
import { MenuPanel, OptionMenuList } from "@/components/OptionMenuList";
import { MenuItem, MenuSeparator, Popover, cx } from "@/components/ui";
import { PERMISSION_MODES, permissionModeOption } from "@/lib/domain/permissions";
import { formatTokens } from "@/lib/format";
import { useFocusedSessionControls, type SessionControls } from "@/features/session/sessionControls";
import { BAR_BUTTON, Divider, Dot, useMenuOpen } from "./parts";
import { meterClasses, meterPercent, meterState } from "./statusBarState";
import { UsageMeter } from "./UsageMeter";

const PERMISSION_ITEMS = PERMISSION_MODES.map((p) => ({
  key: p.key,
  label: p.label,
  hint: p.hint,
  danger: p.danger,
}));

/** The focused chat pane's controls; absent for PTY, terminal and viewer tabs. */
export function SessionControlsCluster({
  workstreamId,
  compact,
}: {
  workstreamId: string | null;
  compact: boolean;
}) {
  const controls = useFocusedSessionControls(workstreamId);
  if (!controls) return null;
  const { context } = controls;
  // TODO(statusbar): 5h/wk need account usage windows; claude's rate_limit_event (dropped, driver.rs:619) has no utilization.
  const percent = context ? meterPercent(context.tokens, context.window) : null;
  return (
    <div className="flex min-w-0 items-center gap-3">
      {context && percent !== null && (
        <>
          <ContextMeter context={context} percent={percent} />
          <Divider />
        </>
      )}
      {controls.planning ? (
        <PermissionPill permissionKey="plan" interactive={false} setByAgent onPick={() => {}} />
      ) : (
        controls.permissionKey !== null && (
          <PermissionPill
            permissionKey={controls.permissionKey}
            interactive={controls.interactive}
            onPick={controls.actions.setPermissionMode}
          />
        )
      )}
      {/* Model and effort read as one setting, so they sit closer than the cluster's gap. */}
      <div className="flex min-w-0 items-center">
        <ModelPicker controls={controls} compact={compact} />
        {controls.effort && (
          <EffortPicker
            effort={controls.effort}
            interactive={controls.interactive}
            onPick={controls.actions.pickEffort}
          />
        )}
      </div>
    </div>
  );
}

function ContextMeter({
  context,
  percent,
}: {
  context: NonNullable<SessionControls["context"]>;
  percent: number;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const used = Math.min(1, Math.max(0, context.tokens / context.window));
  const { fill } = meterClasses(meterState(percent));
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-label={`${context.title} (${percent}%)`}
        className={cx(BAR_BUTTON, "px-1")}
      >
        <UsageMeter label="ctx" percent={percent} title={context.title} />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="w-64 p-3"
      >
        <div className="flex items-baseline justify-between">
          <span className="text-body font-medium text-ink">Context</span>
          <span className="text-meta text-ink-dim">
            {formatTokens(context.tokens)} / {formatTokens(context.window)}
          </span>
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-edge">
          <div className={cx("h-full rounded-full", fill)} style={{ width: `${used * 100}%` }} />
        </div>
        <dl className="mt-2.5 space-y-1 text-ui">
          <div className="flex justify-between">
            <dt className="text-ink-dim">Used</dt>
            <dd className="text-ink">{(used * 100).toFixed(1)}%</dd>
          </div>
          <div className="flex justify-between">
            <dt className="text-ink-dim">Free space</dt>
            <dd className="text-ink">{((1 - used) * 100).toFixed(1)}%</dd>
          </div>
        </dl>
      </Popover>
    </>
  );
}

// Opens a menu rather than cycling: one step past Plan is Bypass.
function PermissionPill({
  permissionKey,
  interactive,
  setByAgent = false,
  onPick,
}: {
  permissionKey: string;
  interactive: boolean;
  setByAgent?: boolean;
  onPick: (key: string) => void;
}) {
  const [open, setOpen] = useMenuOpen(interactive);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const mode = permissionModeOption(permissionKey);
  const name = `Permission mode: ${mode.label}`;
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={!interactive}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={name}
        title={setByAgent ? `${name}, until the agent finishes planning` : name}
        className={cx(BAR_BUTTON, "bg-accent-soft px-2 font-semibold text-ink enabled:hover:bg-edge")}
      >
        <Dot className={mode.danger ? "bg-bad" : "bg-ink"} />
        {mode.short}
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="min-w-44"
      >
        <MenuPanel aria-label="Permission mode">
          <OptionMenuList
            items={PERMISSION_ITEMS}
            activeKey={mode.key}
            onSelect={(key) => {
              onPick(key);
              setOpen(false);
            }}
          />
        </MenuPanel>
      </Popover>
    </>
  );
}

function ModelPicker({ controls, compact }: { controls: SessionControls; compact: boolean }) {
  const { model, actions, interactive } = controls;
  const [open, setOpen] = useMenuOpen(interactive);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const close = () => setOpen(false);
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={!interactive}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Model: ${model.label}`}
        title={`Model: ${model.label}`}
        className={cx(BAR_BUTTON, "min-w-0 shrink font-medium text-ink", compact && "max-w-32")}
      >
        <span className="truncate">{model.label}</span>
        <Icon name="chevronDown" size="xs" className="text-ink-4" />
      </button>
      <Popover
        open={open}
        onClose={close}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="max-h-[60vh] min-w-44 max-w-[22rem] overflow-y-auto overflow-x-hidden"
      >
        <MenuPanel aria-label="Model">
          <OptionMenuList
            groups={model.groups}
            activeKey={model.activeKey}
            aria-label="Model"
            onSelect={(key) => {
              actions.pickModel(key);
              close();
            }}
          />
          <MenuSeparator />
          <MenuItem
            onClick={() => {
              actions.openHarnessSettings();
              close();
            }}
            className="text-ink-dim hover:text-ink"
          >
            <span className="flex w-full items-center gap-2">
              <span>Harnesses</span>
              <span className="ml-auto shrink-0">
                <Icon name="chevronRight" size="sm" />
              </span>
            </span>
          </MenuItem>
        </MenuPanel>
      </Popover>
    </>
  );
}

function EffortPicker({
  effort,
  interactive,
  onPick,
}: {
  effort: NonNullable<SessionControls["effort"]>;
  interactive: boolean;
  onPick: (key: string) => void;
}) {
  const [open, setOpen] = useMenuOpen(interactive);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const name = `Effort: ${effort.label}`;
  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        disabled={!interactive}
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={name}
        title={`Reasoning effort: ${effort.label}`}
        className={cx(BAR_BUTTON, "pl-1 font-medium text-ink")}
      >
        <Icon name="usage" size="xs" className="text-ink-dim" />
        <span className="whitespace-nowrap">{effort.label}</span>
        <Icon name="chevronDown" size="xs" className="text-ink-4" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="min-w-40"
      >
        <MenuPanel aria-label="Effort">
          <OptionMenuList
            items={effort.items}
            activeKey={effort.activeKey}
            onSelect={(key) => {
              onPick(key);
              setOpen(false);
            }}
          />
        </MenuPanel>
      </Popover>
    </>
  );
}
