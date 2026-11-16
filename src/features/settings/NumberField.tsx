import { useState } from "react";

import { TextInput } from "@/components/ui";

// Commits on blur or Enter; an out-of-range entry snaps back to the saved value.
export function NumberField({
  label,
  value,
  parse,
  onCommit,
  step,
  wide,
}: {
  label: string;
  value: number;
  parse: (n: number) => number | null;
  onCommit: (n: number) => void;
  step?: string;
  wide?: boolean;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const commit = () => {
    if (draft === null) return;
    const next = draft.trim() === "" ? null : parse(Number(draft));
    setDraft(null);
    if (next !== null && next !== value) onCommit(next);
  };
  return (
    <TextInput
      type="number"
      aria-label={label}
      step={step}
      value={draft ?? String(value)}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
      }}
      className={wide ? "w-[140px]" : "w-[96px]"}
    />
  );
}
