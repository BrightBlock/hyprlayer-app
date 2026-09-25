import { useEffect, useId, useRef, type KeyboardEvent } from "react";

import { Icon } from "@/components/Icon";
import type { DocContent } from "@/lib/bindings";
import { useUiStore } from "@/lib/uiStore";
import type { DocHeading } from "./editor/headings";
import { DocProperties } from "./DocProperties";
import { presentKeys, type FmDraft, type SchemaKey } from "./fmDraft";

function SectionHeader({
  id,
  title,
  collapsed,
  onToggle,
}: {
  id: string;
  title: string;
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={!collapsed}
      aria-controls={id}
      className="flex w-full shrink-0 items-center gap-1.5 px-4 py-2 text-label text-ink-dim hover:text-ink"
    >
      <Icon name={collapsed ? "chevronRight" : "chevronDown"} size="xs" />
      {title}
    </button>
  );
}

function ContentsList({
  headings,
  activeHeading,
  onJumpToHeading,
}: {
  headings: DocHeading[];
  activeHeading: number;
  onJumpToHeading: (index: number) => void;
}) {
  const listRef = useRef<HTMLUListElement>(null);

  // Keep the active item on-screen as scrollspy moves it out of the panel's
  // own (independently-scrolled, max-h-[40%]) view. A no-op when it's already
  // visible, e.g. right after a click.
  useEffect(() => {
    const item = listRef.current?.children[activeHeading];
    if (item instanceof HTMLElement) item.scrollIntoView({ block: "nearest" });
  }, [activeHeading]);

  if (headings.length === 0) {
    return <p className="px-4 pb-2 text-ui text-ink-4">Add a ## heading to build contents</p>;
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
    <ul ref={listRef} className="min-h-0 overflow-y-auto pb-2" onKeyDown={handleKeyDown}>
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
  activeHeading: number;
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
  activeHeading,
  onJumpToHeading,
  width,
}: Props) {
  const contentsId = useId();
  const propertiesId = useId();
  const contentsCollapsed = useUiStore((s) => s.docRailContentsCollapsed);
  const toggleContents = useUiStore((s) => s.toggleDocRailContents);
  const propertiesCollapsed = useUiStore((s) => s.docRailPropertiesCollapsed);
  const toggleProperties = useUiStore((s) => s.toggleDocRailProperties);
  const fieldCount = presentKeys(doc.fmKeys, draft);

  return (
    <div className="flex min-h-0 shrink-0 flex-col border-l border-edge bg-panel" style={{ width }}>
      <div className="flex max-h-[40%] shrink-0 flex-col border-b border-edge">
        <SectionHeader id={contentsId} title="Contents" collapsed={contentsCollapsed} onToggle={toggleContents} />
        {!contentsCollapsed && (
          <div id={contentsId} className="min-h-0 flex-1 overflow-y-auto">
            <ContentsList headings={headings} activeHeading={activeHeading} onJumpToHeading={onJumpToHeading} />
          </div>
        )}
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        <SectionHeader
          id={propertiesId}
          title="Properties"
          collapsed={propertiesCollapsed}
          onToggle={toggleProperties}
        />
        {!propertiesCollapsed && (
          <div id={propertiesId} className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
            <DocProperties doc={doc} draft={draft} onChange={onChange} raw={raw} setRaw={setRaw} disabled={disabled} />
          </div>
        )}
      </div>

      <div className="shrink-0 border-t border-edge px-4 py-2 text-meta text-ink-4">
        {fieldCount} field{fieldCount === 1 ? "" : "s"}
      </div>
    </div>
  );
}
