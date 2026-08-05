import { useState } from "react";

import { Button, Dialog, TextInput } from "@/components/ui";
import { ZapIcon } from "@/components/icons";
import { promptInputLabel } from "./prompts";
import { answerPromptArgs, usePromptArgsStore, type PromptArgsRequest } from "./promptsStore";

/**
 * Asks for a custom prompt's `{{input}}` arguments before it runs. One shared
 * dialog serves every surface — ⌘K palette, composer chip, keyboard shortcut —
 * because all three funnel through `runCustomPrompt`. Escape or a backdrop click
 * cancels the run; empty answers are allowed and substitute as empty.
 */
export function PromptArgsDialog() {
  const request = usePromptArgsStore((s) => s.request);
  return (
    <Dialog
      open={request !== null}
      onClose={() => answerPromptArgs(null)}
      ariaLabel="Prompt arguments"
      className="max-w-sm"
    >
      {request && <ArgsForm key={request.id} request={request} />}
    </Dialog>
  );
}

function ArgsForm({ request }: { request: PromptArgsRequest }) {
  const [values, setValues] = useState<Record<string, string>>(request.initial);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        answerPromptArgs(values);
      }}
    >
      <div className="mb-3 flex items-center gap-2">
        <ZapIcon size={14} className="shrink-0 text-accent" />
        <h2 className="min-w-0 truncate text-sm font-semibold tracking-wide text-ink">
          {request.name}
        </h2>
      </div>

      <div className="space-y-3">
        {request.inputs.map((input, i) => (
          <label key={input.token} className="block space-y-1.5">
            <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
              {promptInputLabel(input)}
            </span>
            <TextInput
              autoFocus={i === 0}
              spellCheck={false}
              value={values[input.token] ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, [input.token]: e.target.value }))}
            />
          </label>
        ))}
      </div>

      <div className="mt-4 flex items-center justify-end gap-2">
        <Button type="button" onClick={() => answerPromptArgs(null)}>
          Cancel
        </Button>
        <Button type="submit" variant="primary">
          Run
        </Button>
      </div>
    </form>
  );
}
