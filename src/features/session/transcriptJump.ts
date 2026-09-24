import { create } from "zustand";

export type TranscriptJump = { toolUseId: string } | { itemId: string };

type PendingJump = { sessionId: string; target: TranscriptJump; nonce: number };

// Lets a panel outside the chat (the plan tracker) scroll that session's transcript.
export const useTranscriptJumpStore = create<{ pending: PendingJump | null }>(() => ({ pending: null }));

let nonce = 0;

export function requestTranscriptJump(sessionId: string, target: TranscriptJump): void {
  useTranscriptJumpStore.setState({ pending: { sessionId, target, nonce: nonce++ } });
}

export function clearTranscriptJump(): void {
  useTranscriptJumpStore.setState({ pending: null });
}
