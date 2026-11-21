import { useNavigate, useRouter } from "@tanstack/react-router";

import { openExternalUrl } from "@/lib/browser";
import { Icon } from "@/components/Icon";
import type { RelatedLink, RelatedTarget } from "@/lib/bindings";
import { prefetchDoc, useRelatedDocs } from "@/lib/queries";
import { docTypeIcon } from "./docTypeIcon";
import { RelatedPicker } from "./RelatedPicker";

type Props = {
  id: string;
  values: string[];
  onChange: (next: string[]) => void;
};

const REMOVE_BTN =
  "shrink-0 rounded p-0.5 text-ink-dim opacity-0 hover:text-ink focus:opacity-100 group-hover:opacity-100";

function RemoveButton({ raw, onRemove }: { raw: string; onRemove: () => void }) {
  return (
    <button type="button" onClick={onRemove} aria-label={`Remove related ${raw}`} className={REMOVE_BTN}>
      <Icon name="close" size="xs" />
    </button>
  );
}

function PlainRow({ value, onRemove }: { value: string; onRemove: () => void }) {
  return (
    <li className="group flex items-center gap-1.5 rounded px-1 py-0.5">
      <span className="min-w-0 flex-1 truncate text-body text-ink-dim">{value}</span>
      <RemoveButton raw={value} onRemove={onRemove} />
    </li>
  );
}

function DocRow({
  raw,
  target,
  onRemove,
}: {
  raw: string;
  target: Extract<RelatedTarget, { kind: "doc" }>;
  onRemove: () => void;
}) {
  const router = useRouter();
  const navigate = useNavigate();
  const search = (prev: Record<string, unknown>) => ({ ...prev, doc: target.id });
  const href = router.buildLocation({ to: "/library", search }).href;
  return (
    <li className="group flex items-center gap-1.5 rounded px-1 py-0.5 hover:bg-panel-2">
      <a
        href={href}
        className="flex min-w-0 flex-1 items-center gap-1.5 truncate text-body text-ink"
        onClick={(e) => {
          e.preventDefault();
          void navigate({ to: "/library", search });
        }}
        onAuxClick={(e) => e.preventDefault()}
        onMouseEnter={() => prefetchDoc(target.id)}
        onFocus={() => prefetchDoc(target.id)}
      >
        <Icon name={docTypeIcon(target.docType)} size="sm" className="shrink-0 text-ink-dim" />
        <span className="min-w-0 flex-1 truncate" title={target.title}>
          {target.title}
        </span>
        {target.docType && <span className="shrink-0 text-meta text-ink-4">{target.docType}</span>}
      </a>
      <RemoveButton raw={raw} onRemove={onRemove} />
    </li>
  );
}

function ExternalRow({
  raw,
  target,
  onRemove,
}: {
  raw: string;
  target: Extract<RelatedTarget, { kind: "external" }>;
  onRemove: () => void;
}) {
  return (
    <li className="group flex items-center gap-1.5 rounded px-1 py-0.5 hover:bg-panel-2">
      <a
        href={target.url}
        className="flex min-w-0 flex-1 items-center gap-1.5 truncate text-body text-info underline"
        onClick={(e) => {
          e.preventDefault();
          void openExternalUrl(target.url);
        }}
      >
        <Icon name={docTypeIcon("external")} size="sm" className="shrink-0 text-ink-dim" />
        <span className="min-w-0 flex-1 truncate">{target.url}</span>
      </a>
      <RemoveButton raw={raw} onRemove={onRemove} />
    </li>
  );
}

function UnresolvedRow({
  raw,
  target,
  onRemove,
}: {
  raw: string;
  target: Extract<RelatedTarget, { kind: "unresolved" }>;
  onRemove: () => void;
}) {
  const title = target.ambiguous ? `Ambiguous: ${raw}` : `Not found: ${raw}`;
  return (
    <li className="flex items-center gap-1.5 rounded px-1 py-0.5" title={title}>
      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-body text-ink-4">{raw}</span>
      <button
        type="button"
        onClick={onRemove}
        aria-label={`Remove related ${raw}`}
        className="shrink-0 text-ui text-ink-dim hover:text-ink"
      >
        Remove
      </button>
    </li>
  );
}

function ResolvedRow({ link, onRemove }: { link: RelatedLink; onRemove: () => void }) {
  if (link.target.kind === "doc") return <DocRow raw={link.raw} target={link.target} onRemove={onRemove} />;
  if (link.target.kind === "external") return <ExternalRow raw={link.raw} target={link.target} onRemove={onRemove} />;
  return <UnresolvedRow raw={link.raw} target={link.target} onRemove={onRemove} />;
}

export function RelatedField({ id, values, onChange }: Props) {
  const { data: links, isPlaceholderData, isLoading, isError } = useRelatedDocs(id, values);
  // A refetch after a pick or removal must never show a stale link as clickable.
  const stale = isLoading || isError || isPlaceholderData;

  const remove = (value: string) => onChange(values.filter((v) => v !== value));

  const linkedIds = (links ?? [])
    .filter((l): l is RelatedLink & { target: Extract<RelatedTarget, { kind: "doc" }> } => l.target.kind === "doc")
    .map((l) => l.target.id);

  return (
    <div>
      {values.length === 0 ? (
        <p className="text-body text-ink-4">—</p>
      ) : (
        <ul id="fm-related" aria-labelledby="fm-related-label" className="space-y-0.5">
          {values.map((value) => {
            const link = stale ? undefined : links?.find((l) => l.raw === value);
            return link ? (
              <ResolvedRow key={value} link={link} onRemove={() => remove(value)} />
            ) : (
              <PlainRow key={value} value={value} onRemove={() => remove(value)} />
            );
          })}
        </ul>
      )}
      <RelatedPicker
        id={id}
        excludeIds={linkedIds}
        onPick={(value) => {
          if (!values.includes(value)) onChange([...values, value]);
        }}
      />
    </div>
  );
}
