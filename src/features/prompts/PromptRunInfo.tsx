import { Card, CardTitle, TextInput } from "@/components/ui";
import { promptInputLabel, promptInputs, usedVars } from "./prompts";

export function PromptRunInfo({
  body,
  builtinName,
  slug,
}: {
  body: string;
  /** Set for built-ins, which run as a stage and never ask for input. */
  builtinName: string | null;
  slug: string;
}) {
  const inputs = builtinName ? [] : promptInputs(body);
  const vars = usedVars(body);

  return (
    <Card>
      <CardTitle>When it runs</CardTitle>
      <div className="space-y-3 text-ui text-ink-dim">
        {builtinName ? (
          <p>Runs as the {builtinName} stage — no inputs.</p>
        ) : inputs.length > 0 ? (
          <>
            <p>Hyprlayer asks for each input once, then sends the prompt.</p>
            <div className="space-y-2">
              {inputs.map((i) => (
                <label key={i.token} className="grid grid-cols-[10rem_minmax(0,1fr)] items-center gap-3">
                  <span className="truncate text-body text-ink">{promptInputLabel(i)}</span>
                  <TextInput disabled placeholder={promptInputLabel(i)} />
                </label>
              ))}
            </div>
            {inputs.length === 1 && <p>Text typed after /{slug} fills it.</p>}
          </>
        ) : (
          <p>Sends immediately. Text typed after /{slug} is appended.</p>
        )}
        {vars.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span>Filled from the workstream</span>
            {vars.map((v) => (
              <code key={v.token} className="rounded bg-brand/15 px-1.5 py-0.5 text-ui-sm text-brand">
                {v.token}
              </code>
            ))}
          </div>
        )}
      </div>
    </Card>
  );
}
