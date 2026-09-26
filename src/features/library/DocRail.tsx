import { useEffect, useId, useRef, useSyncExternalStore, type KeyboardEvent, type ReactNode } from "react";

import { Icon } from "@/components/Icon";
import type { DocContent } from "@/lib/bindings";
import { useUiStore } from "@/lib/uiStore";
import type { ActiveHeading } from "./editor/contentsNav";
import type { DocHeading } from "./editor/headings";
import { DocProperties } from "./DocProperties";
import { presentKeys, type FmDraft, type SchemaKey } from "./fmDraft";

// Same shape as the workstream sidebar's panels: an open section shares the rail, a collapsed one yields it.
function RailSection({
  title,
  count,
  collapsed,
  onToggle,
  children,
}: {
  title: string;
  count: number;
  collapsed: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <div className={`flex flex-col border-b border-edge ${collapsed ? "shrink-0" : "min-h-0 flex-1"}`}>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        aria-controls={id}
        className="flex h-9 shrink-0 items-center gap-2 px-3 text-left text-ink-dim hover:text-ink"
      >
        <span className="flex w-3 shrink-0 justify-center">
          <Icon name={collapsed ? "chevronRight" : "chevronDown"} size="xs" />
        </span>
        <span className="text-label">{title}</span>
        {count > 0 && <span className="ml-auto text-meta">{count}</span>}
      </button>
      {!collapsed && (
        <div id={id} className="min-h-0 flex-1 overflow-y-auto border-t border-edge">
          {children}
        </div>
      )}
    </div>
  );
}

function ContentsList({
  headings,
  active,
  onJumpToHeading,
}: {
  headings: DocHeading[];
  active: ActiveHeading;
  onJumpToHeading: (index: number) => void;
}) {
  const activeHeading = useSyncExternalStore(active.subscribe, active.get);
  const listRef = useRef<HTMLUListElement>(null);

  // Keep the active item visible in the section's own scroll area as the scrollspy moves it.
  useEffect(() => {
    const item = listRef.current?.children[activeHeading];
    if (item instanceof HTMLElement) item.scrollIntoView({ block: "nearest" });
  }, [activeHeading]);

  if (headings.length === 0) {
    return <p className="px-4 py-2 text-ui text-ink-4">Add a ## heading to build contents</p>;
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLUListElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const buttons = Array.from(e.currentTarget.querySelectorAll("button"));
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = current + (e.key === "ArrowDown" ? 1 : -1);
    buttons[Math.max(0, Math.min(buttons.length - 1, next))]?.focus();
  };

  return (
    <ul ref={listRef} className="py-1" onKeyDown={handleKeyDown}>
      {headings.map((h, i) => (
        <li key={h.from}>
          <button
            type="button"
            title={h.text}
            onClick={() => onJumpToHeading(i)}
            style={{ paddingLeft: 16 + 12 * (h.level - 2) }}
            className={`block w-full truncate py-1 pr-4 text-left text-ui text-ink-dim hover:text-ink ${
              i === activeHeading
                ? "relative text-ink font-semibold before:absolute before:inset-y-0 before:left-0 before:w-[2px] before:bg-accent"
                : ""
            }`}
          >
            {h.text}
          </button>
        </li>
      ))}
    </ul>
  );
}

type Props = {
  doc: DocContent;
  draft: FmDraft;
  onChange: (key: SchemaKey, next: string | string[]) => void;
  raw: string | null;
  setRaw: (text: string | null) => void;
  disabled: boolean;
  headings: DocHeading[];
  active: ActiveHeading;
  onJumpToHeading: (index: number) => void;
  width: number;
};

export function DocRail({
  doc,
  draft,
  onChange,
  raw,
  setRaw,
  disabled,
  headings,
  active,
  onJumpToHeading,
  width,
}: Props) {
  const contentsCollapsed = useUiStore((s) => s.docRailContentsCollapsed);
  const toggleContents = useUiStore((s) => s.toggleDocRailContents);
  const propertiesCollapsed = useUiStore((s) => s.docRailPropertiesCollapsed);
  const toggleProperties = useUiStore((s) => s.toggleDocRailProperties);

  return (
    <div className="flex min-h-0 shrink-0 flex-col border-l border-edge bg-panel" style={{ width }}>
      <RailSection title="Contents" count={headings.length} collapsed={contentsCollapsed} onToggle={toggleContents}>
        <ContentsList headings={headings} active={active} onJumpToHeading={onJumpToHeading} />
      </RailSection>
      <RailSection
        title="Properties"
        count={presentKeys(doc.fmKeys, draft)}
        collapsed={propertiesCollapsed}
        onToggle={toggleProperties}
      >
        <div className="px-4 pb-4 pt-3">
          <DocProperties doc={doc} draft={draft} onChange={onChange} raw={raw} setRaw={setRaw} disabled={disabled} />
        </div>
      </RailSection>
    </div>
  );
}
