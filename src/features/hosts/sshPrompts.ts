import { create } from "zustand";

import { commands, events, type SshPrompt } from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";

// The backend's queue of questions ssh is waiting on; every change arrives as the whole list.
export const useSshPrompts = create<{ prompts: SshPrompt[] }>(() => ({ prompts: [] }));

let heard = false;

function load(): void {
  void unwrap(commands.listSshPrompts())
    .then((prompts) => {
      if (!heard) useSshPrompts.setState({ prompts });
    })
    .catch((e) => log.warn("ssh", "pending ssh prompts did not load", e));
}

export const sshPromptBridge: EventBridge = () => {
  const listening = events.sshPromptsChanged.listen((event) => {
    heard = true;
    useSshPrompts.setState({ prompts: event.payload.prompts });
  });
  // Only once the listener is live: earlier, a change could fall between the two.
  void listening.then(load);
  return [listening];
};

/**
 * `null` cancels, which stops that ssh run rather than answering "no". The
 * question leaves the queue at once, which also drops anything typed into it.
 */
export function answerSshPrompt(id: string, answer: string | null): void {
  useSshPrompts.setState((s) => ({ prompts: s.prompts.filter((p) => p.id !== id) }));
  void unwrap(commands.answerSshPrompt(id, answer)).catch((e) => {
    log.warn("ssh", "answering an ssh prompt failed", e);
    // Still waiting in the backend, so it comes back.
    heard = false;
    load();
  });
}
