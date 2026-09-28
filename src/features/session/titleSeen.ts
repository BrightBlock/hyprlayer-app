// Leaf module: a title the server just sent must never be pushed right back to it.
const seen = new Map<string, string>();

export function markTitleSeen(sessionId: string, title: string): void {
  seen.set(sessionId, title);
}

export function takeSeenTitle(sessionId: string): string | undefined {
  const title = seen.get(sessionId);
  seen.delete(sessionId);
  return title;
}
