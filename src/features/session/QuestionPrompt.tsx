import { useState } from "react";

import { Button } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { buildQuestionAnswers, type PendingQuestion } from "./transcript";

export function QuestionPrompt({
  pending,
  onSubmit,
}: {
  pending: PendingQuestion;
  onSubmit: (answers: Record<string, string>) => void;
}) {
  const [selections, setSelections] = useState<Record<string, string[]>>({});
  const [custom, setCustom] = useState<Record<string, string>>({});
  const [step, setStep] = useState(0);
  const [submitted, setSubmitted] = useState(false);

  const send = (answers: Record<string, string>) => {
    if (submitted) return;
    setSubmitted(true);
    onSubmit(answers);
  };

  const total = pending.questions.length;
  const question = pending.questions[step];

  if (!question) {
    return (
      <div className="mb-2 flex items-center justify-between gap-2 rounded-card border border-q-card-border bg-q-card-bg p-3.5 shadow-card">
        <span className="text-body text-ink-dim">
          The agent asked a question that couldn’t be displayed.
        </span>
        <Button disabled={submitted} onClick={() => send({})}>
          Dismiss
        </Button>
      </div>
    );
  }

  const chosen = selections[question.question] ?? [];
  const otherText = custom[question.question];
  const otherOpen = otherText !== undefined;
  const currentAnswered = chosen.length > 0 || (otherText ?? "").trim().length > 0;
  const isLast = step === total - 1;

  const closeOther = () =>
    setCustom((prev) => {
      if (!(question.question in prev)) return prev;
      const next = { ...prev };
      delete next[question.question];
      return next;
    });

  const toggle = (label: string) => {
    setSelections((prev) => {
      const current = prev[question.question] ?? [];
      if (question.multiSelect) {
        const next = current.includes(label)
          ? current.filter((l) => l !== label)
          : [...current, label];
        return { ...prev, [question.question]: next };
      }
      return { ...prev, [question.question]: [label] };
    });
    if (!question.multiSelect) closeOther();
  };

  const toggleOther = () => {
    if (otherOpen) {
      closeOther();
      return;
    }
    setCustom((prev) => ({ ...prev, [question.question]: "" }));
    if (!question.multiSelect) setSelections((prev) => ({ ...prev, [question.question]: [] }));
  };

  const submitAll = () => send(buildQuestionAnswers(pending.questions, selections, custom));

  const advance = () => {
    if (!currentAnswered) return;
    if (isLast) submitAll();
    else setStep((s) => s + 1);
  };

  const otherDot = (mt: string) => (
    <span
      className={`flex h-4 w-4 shrink-0 items-center justify-center border ${mt} ${
        question.multiSelect ? "rounded" : "rounded-full"
      } ${otherOpen ? "border-accent bg-accent" : "border-radio-border"}`}
      aria-hidden="true"
    >
      {otherOpen && (
        <Icon name="check" size="xs" className="text-accent-ink" />
      )}
    </span>
  );

  return (
    <div className="mb-2 rounded-card border border-q-card-border bg-q-card-bg p-3.5 shadow-card">
      <div className="mb-2.5 flex items-center gap-2 text-q-label">
        <Icon name="question" className="text-warn" />
        <span className="text-body font-medium">The agent is asking</span>
        {total > 1 && (
          <span className="ml-auto text-ui font-normal tabular-nums text-ink-dim">
            {step + 1} of {total}
          </span>
        )}
      </div>

      {question.header && (
        <span className="mb-1 block text-label text-ink-dim">
          {question.header}
        </span>
      )}
      <p className="text-body font-medium text-ink">{question.question}</p>
      {question.multiSelect && (
        <p className="mt-0.5 text-ui text-ink-dim">Select all that apply.</p>
      )}
      <div className="mt-2 space-y-1.5">
        {question.options.map((opt) => {
          const selected = chosen.includes(opt.label);
          return (
            <button
              key={opt.label}
              type="button"
              disabled={submitted}
              onClick={() => toggle(opt.label)}
              aria-pressed={selected}
              className={`flex w-full items-start gap-2.5 rounded-control border px-3 py-2 text-left disabled:opacity-50 ${
                selected
                  ? "border-accent bg-accent/15"
                  : "border-edge bg-panel-2 hover:border-edge hover:bg-edge/50"
              }`}
            >
              <span
                className={`mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center border ${
                  question.multiSelect ? "rounded" : "rounded-full"
                } ${selected ? "border-accent bg-accent" : "border-radio-border"}`}
                aria-hidden="true"
              >
                {selected && (
                  <Icon name="check" size="xs" className="text-accent-ink" />
                )}
              </span>
              <span className="min-w-0">
                <span className="block text-body text-ink">{opt.label}</span>
                {opt.description && (
                  <span className="mt-0.5 block text-ui text-ink-dim">
                    {opt.description}
                  </span>
                )}
              </span>
            </button>
          );
        })}
        <div
          className={`rounded-control border ${
            otherOpen
              ? "border-accent bg-accent/15"
              : "border-edge bg-panel-2 hover:border-edge hover:bg-edge/50"
          }`}
        >
          {otherOpen ? (
            <div className="flex items-center gap-2.5 px-3 py-2">
              <button
                type="button"
                disabled={submitted}
                onClick={toggleOther}
                aria-pressed={otherOpen}
                aria-label="Remove custom answer"
                className="shrink-0 disabled:opacity-50"
              >
                {otherDot("")}
              </button>
              <input
                autoFocus
                disabled={submitted}
                value={otherText ?? ""}
                aria-label="Custom answer"
                onChange={(e) =>
                  setCustom((prev) => ({ ...prev, [question.question]: e.target.value }))
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    advance();
                  }
                }}
                placeholder="Type your answer…"
                className="min-w-0 flex-1 bg-transparent text-body text-ink placeholder:text-ink-dim focus:outline-none disabled:opacity-50"
              />
            </div>
          ) : (
            <button
              type="button"
              disabled={submitted}
              onClick={toggleOther}
              aria-pressed={otherOpen}
              className="flex w-full items-start gap-2.5 px-3 py-2 text-left disabled:opacity-50"
            >
              {otherDot("mt-0.5")}
              <span className="block text-body text-ink">Other…</span>
            </button>
          )}
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2">
        {step > 0 && (
          <Button disabled={submitted} onClick={() => setStep((s) => s - 1)}>
            Back
          </Button>
        )}
        <Button variant="primary" disabled={!currentAnswered || submitted} onClick={advance}>
          {isLast ? (total > 1 ? "Submit answers" : "Submit answer") : "Next"}
        </Button>
        <Button disabled={submitted} onClick={() => send({})}>
          Skip
        </Button>
      </div>
    </div>
  );
}
