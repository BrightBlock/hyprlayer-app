import { useState, type FormEvent, type ReactNode } from "react";

import type { SshPrompt, SshPromptKind } from "@/lib/bindings";
import { hostName } from "@/lib/domain/hosts";
import { useEscape } from "@/lib/keybindings/registry";
import { Button, cx, Dialog, Spinner, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { answerSshPrompt, useSshPrompts } from "./sshPrompts";

// A stray backdrop click must not cancel: a cancelled sign-in stops the
// reconnect loop. Escape and Cancel still do.
const NOOP = () => {};

const TITLE: Record<SshPromptKind, (name: string) => string> = {
  hostKey: (name) => `Trust ${name}?`,
  secret: (name) => `Sign in to ${name}`,
  confirm: (name) => name,
  notice: (name) => name,
};

/** Whatever ssh is waiting on the user for, one question at a time. */
export function SshPromptDialog() {
  const prompt = useSshPrompts((s) => s.prompts[0] ?? null);
  if (!prompt) return null;
  return <PromptDialog key={prompt.id} prompt={prompt} />;
}

function PromptDialog({ prompt }: { prompt: SshPrompt }) {
  const [sent, setSent] = useState(false);
  const send = (answer: string | null) => {
    if (sent) return;
    setSent(true);
    answerSshPrompt(prompt.id, answer);
  };
  // Registered after the Dialog's own (effects run child-first), so it wins.
  useEscape(() => send(null));
  const title = TITLE[prompt.kind](hostName(prompt.host));

  return (
    <Dialog
      open
      onClose={NOOP}
      size="sm"
      ariaLabel={title}
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Icon name="lock" className="text-accent" />
          <span className="min-w-0 truncate">{title}</span>
        </span>
      }
    >
      {prompt.kind === "secret" ? (
        <SecretForm prompt={prompt} sent={sent} onSend={send} />
      ) : prompt.kind === "hostKey" ? (
        <HostKeyCheck prompt={prompt} sent={sent} onSend={send} />
      ) : prompt.kind === "confirm" ? (
        <>
          <Message text={prompt.message} />
          <Actions>
            <Button onClick={() => send("no")} disabled={sent}>
              No
            </Button>
            <Button variant="primary" onClick={() => send("yes")} disabled={sent} autoFocus>
              Yes
            </Button>
          </Actions>
        </>
      ) : (
        <>
          <div className="flex items-start gap-2">
            <Spinner size="sm" />
            <Message text={prompt.message} />
          </div>
          <Actions>
            <Button onClick={() => send(null)} disabled={sent} autoFocus>
              Cancel
            </Button>
          </Actions>
        </>
      )}
    </Dialog>
  );
}

type Props = { prompt: SshPrompt; sent: boolean; onSend: (answer: string | null) => void };

function SecretForm({ prompt, sent, onSend }: Props) {
  const [value, setValue] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!value) return;
    const answer = value;
    setValue("");
    onSend(answer);
  };

  return (
    <form onSubmit={submit}>
      <label className="block space-y-1.5">
        <span className="block whitespace-pre-wrap break-words text-ui text-ink-dim">
          {prompt.message.trim().replace(/:$/, "")}
        </span>
        <TextInput
          type="password"
          autoFocus
          autoComplete="off"
          spellCheck={false}
          value={value}
          disabled={sent}
          aria-invalid={prompt.retry}
          className={cx(prompt.retry && "border-bad/60 focus:border-bad")}
          onChange={(e) => setValue(e.target.value)}
        />
      </label>
      {prompt.retry && (
        <p role="alert" className="mt-1.5 text-ui text-bad-ink">
          Not accepted. Try again.
        </p>
      )}
      <Actions>
        <Button type="button" onClick={() => onSend(null)} disabled={sent}>
          Cancel
        </Button>
        <Button type="submit" variant="primary" disabled={!value} busy={sent}>
          Continue
        </Button>
      </Actions>
    </form>
  );
}

function HostKeyCheck({ prompt, sent, onSend }: Props) {
  return (
    <>
      {prompt.fingerprint ? (
        <div className="space-y-2.5">
          <p className="text-body leading-prose text-ink-dim">
            First connection to{" "}
            <span className="text-ink">{prompt.keyHost ?? prompt.host.destination}</span>. Trust it
            only if this fingerprint matches that host's key.
          </p>
          <div className="selectable break-all rounded-control border border-edge bg-panel-2 px-2.5 py-2 font-mono text-ui text-ink">
            {prompt.keyType && <span className="text-ink-dim">{prompt.keyType} </span>}
            {prompt.fingerprint}
          </div>
        </div>
      ) : (
        <Message text={prompt.message} />
      )}
      <Actions>
        {/* Focused, so a reflexive Enter declines rather than trusts. */}
        <Button onClick={() => onSend(null)} disabled={sent} autoFocus>
          Cancel
        </Button>
        <Button variant="primary" onClick={() => onSend("yes")} disabled={sent}>
          Trust
        </Button>
      </Actions>
    </>
  );
}

function Message({ text }: { text: string }) {
  return (
    <p className="selectable min-w-0 whitespace-pre-wrap break-words text-body leading-prose text-ink-dim">
      {text.trim()}
    </p>
  );
}

function Actions({ children }: { children: ReactNode }) {
  return <div className="mt-4 flex items-center justify-end gap-2">{children}</div>;
}
