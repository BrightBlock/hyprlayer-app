import { useEffect, useRef, useState } from "react";

import type { OpencodeSignInAttempt, OpencodeSignInMethod } from "@/lib/bindings";
import { openExternalUrl } from "@/lib/browser";
import {
  cancelOpencodeSignIn,
  useBeginOpencodeSignIn,
  useCompleteOpencodeSignIn,
  useOpencodeSignInStatus,
} from "@/lib/queries";
import { reauthSucceeded } from "@/lib/reauth";
import { Button, Dialog, ErrorNote, Spinner, TextInput } from "@/components/ui";

/**
 * Signs in to a provider through hyprlayer's own OpenCode server, so the credential lands in
 * its data dir. The browser opens here; on a remote server the browser cannot reach that
 * server's sign-in callback, so the attempt also takes the address the browser lands on.
 */
export function OpenCodeSignInDialog({
  providerName,
  methods,
  onSignedIn,
  onClose,
}: {
  providerName: string;
  methods: OpencodeSignInMethod[];
  onSignedIn: () => void;
  /** Dismissed before the sign-in finished; an attempt under way is cancelled. */
  onClose: () => void;
}) {
  const begin = useBeginOpencodeSignIn();
  const complete = useCompleteOpencodeSignIn();
  const [attempt, setAttempt] = useState<OpencodeSignInAttempt | null>(null);
  const [pasted, setPasted] = useState("");
  const status = useOpencodeSignInStatus(attempt?.provider ?? "", attempt?.attemptId ?? null);
  const state = status.data?.state;
  const open = useRef<OpencodeSignInAttempt | null>(null);
  open.current = state === "pending" || !state ? attempt : null;

  useEffect(
    () => () => {
      if (open.current) cancelOpencodeSignIn(open.current.provider, open.current.attemptId);
    },
    [],
  );

  useEffect(() => {
    if (state !== "complete") return;
    reauthSucceeded("opencode");
    onSignedIn();
  }, [state, onSignedIn]);

  async function start(method: OpencodeSignInMethod) {
    const started = await begin.mutateAsync({ provider: method.provider, methodId: method.methodId });
    setAttempt(started);
    if (started.url) await openExternalUrl(started.url);
  }

  function finish() {
    if (!attempt || !pasted.trim()) return;
    complete.mutate({ provider: attempt.provider, attemptId: attempt.attemptId, code: pasted.trim() });
  }

  const failed = status.data?.state === "failed" ? status.data.message : null;
  const takesInput = attempt && (attempt.needsCode || attempt.acceptsPaste);

  return (
    <Dialog open onClose={onClose} title={`Sign in to ${providerName}`} size="md">
      {!attempt ? (
        <div className="space-y-2">
          <p className="text-ui text-ink-dim">
            The sign-in opens in your browser. Its credential stays with Hyprlayer’s OpenCode server.
          </p>
          {methods.map((method) => (
            <Button
              key={method.methodId}
              className="w-full justify-center"
              busy={begin.isPending && begin.variables?.methodId === method.methodId}
              onClick={() => void start(method)}
            >
              {method.label}
            </Button>
          ))}
          {begin.isError && <ErrorNote error={begin.error} onDismiss={() => begin.reset()} />}
        </div>
      ) : (
        <div className="space-y-3">
          {attempt.instructions && <p className="text-body text-ink">{attempt.instructions}</p>}
          {attempt.url && (
            <Button onClick={() => void openExternalUrl(attempt.url)}>Open the sign-in page again</Button>
          )}
          {!failed && (
            <p className="flex items-center gap-2 text-ui text-ink-dim">
              <Spinner /> Waiting for the sign-in to finish
            </p>
          )}
          {failed && <p className="text-ui text-bad">{failed}</p>}
          {takesInput && !failed && (
            <div>
              <label className="mb-1 block text-ui font-medium text-ink-dim">
                {attempt.needsCode
                  ? "Code from the sign-in page"
                  : "If the browser ends on a page that won’t load, paste its address here"}
              </label>
              <div className="flex gap-2">
                <TextInput
                  value={pasted}
                  onChange={(e) => setPasted(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") finish();
                  }}
                />
                <Button variant="primary" busy={complete.isPending} disabled={!pasted.trim()} onClick={finish}>
                  Finish
                </Button>
              </div>
            </div>
          )}
          {complete.isError && <ErrorNote error={complete.error} onDismiss={() => complete.reset()} />}
          {status.isError && <ErrorNote error={status.error} />}
        </div>
      )}
    </Dialog>
  );
}
