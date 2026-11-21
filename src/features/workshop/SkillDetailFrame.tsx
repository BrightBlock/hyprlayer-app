import type { ReactNode } from "react";

import { openExternalUrl } from "@/lib/browser";
import { Icon } from "@/components/Icon";
import { IconButton, TabStrip } from "@/components/ui";
import { useUiStore } from "@/lib/uiStore";

// Header, doc and rail share one measure, so the rail sits beside the doc at either width.
const columnClass = (wide: boolean) => `mx-auto w-full px-8 ${wide ? "max-w-none" : "max-w-[1280px]"}`;

export function SkillDetailFrame<K extends string>({
  header,
  tabs,
  tab,
  onTab,
  body,
  rail,
}: {
  header: ReactNode;
  tabs: ReadonlyArray<{ key: K; label: ReactNode }>;
  tab: K;
  onTab: (tab: K) => void;
  body: ReactNode;
  rail: ReactNode;
}) {
  const wide = useUiStore((s) => s.skillDetailWide);
  const setWide = useUiStore((s) => s.setSkillDetailWide);
  const column = columnClass(wide);
  return (
    <div className="@container flex h-full min-w-0 flex-col overflow-hidden">
      <div className="shrink-0 border-b border-edge">
        <div className={`${column} pt-6`}>
          {header}
          <div className="mt-4 flex items-center gap-3">
            <TabStrip tabs={tabs} value={tab} onChange={onTab} className="min-w-0 flex-1 border-b-0" />
            <IconButton
              aria-label={wide ? "Narrow view" : "Wide view"}
              title={wide ? "Narrow view" : "Wide view"}
              onClick={() => setWide(!wide)}
            >
              <Icon name={wide ? "narrow" : "widen"} size="sm" />
            </IconButton>
          </div>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className={`${column} flex flex-col @[720px]:flex-row @[720px]:items-start`}>
          <div className="min-w-0 flex-1 py-6 @[720px]:pr-10">{body}</div>
          <aside
            aria-label="About this skill"
            className="flex shrink-0 flex-col gap-6 border-t border-edge py-6 @[720px]:sticky @[720px]:top-0 @[720px]:w-[248px] @[720px]:border-t-0"
          >
            {rail}
          </aside>
        </div>
      </div>
    </div>
  );
}

export function RailSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2 text-label text-ink-dim">{title}</div>
      <dl className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-1.5">{children}</dl>
    </section>
  );
}

export function RailRow({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <>
      <dt className="text-ui text-ink-dim">{label}</dt>
      <dd className={`m-0 break-words text-ui text-ink ${mono ? "font-mono text-meta [overflow-wrap:anywhere]" : ""}`}>
        {value}
      </dd>
    </>
  );
}

export function RepoLink({ repo }: { repo: string }) {
  return (
    <button
      type="button"
      onClick={() => void openExternalUrl(`https://github.com/${repo}`).catch(() => {})}
      className="inline-flex max-w-full items-center gap-1 text-left text-ink underline decoration-edge underline-offset-2 hover:decoration-ink-dim"
    >
      <span className="truncate">{repo}</span>
      <Icon name="external" size="xs" />
    </button>
  );
}
