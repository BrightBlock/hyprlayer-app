import { useRef, useState } from "react";

import { Icon, stageIconName } from "@/components/Icon";
import { MenuPanel, OptionMenuList } from "@/components/OptionMenuList";
import { Popover } from "@/components/ui";
import { MODE_STAGES, MODE_STYLES, stageLabel, type StageKey } from "@/lib/domain/stages";

const ITEMS = MODE_STAGES.map((s) => ({
  key: s.key,
  label: s.label,
  icon: (
    <span className={MODE_STYLES[s.key].text}>
      <Icon name={stageIconName(s.key)} size="sm" />
    </span>
  ),
}));

const TILE = "flex h-8 w-8 shrink-0 items-center justify-center rounded-md";

/** The tab's agent mode as a coloured icon; it can change only until the session starts. */
export function AgentModePicker({
  mode,
  started,
  onChange,
}: {
  mode: StageKey;
  started: boolean;
  onChange: (mode: StageKey) => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const name = `Agent mode: ${stageLabel(mode)}`;
  const tone = MODE_STYLES[mode].badge;

  if (started) {
    return (
      <span role="img" aria-label={name} title={name} className={`${TILE} ${tone}`}>
        <Icon name={stageIconName(mode)} />
      </span>
    );
  }

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={name}
        title={name}
        className={`${TILE} ${tone} transition-opacity hover:opacity-80 focus:outline-none focus-visible:opacity-80`}
      >
        <Icon name={stageIconName(mode)} />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="start"
        className="min-w-44"
      >
        <MenuPanel aria-label="Agent mode">
          <OptionMenuList
            items={ITEMS}
            activeKey={mode}
            onSelect={(key) => {
              onChange(key as StageKey);
              setOpen(false);
            }}
          />
        </MenuPanel>
      </Popover>
    </>
  );
}
