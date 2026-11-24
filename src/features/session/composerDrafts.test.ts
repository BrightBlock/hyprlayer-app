import { beforeEach, describe, expect, it } from "vitest";

import { composerDraft, returnQueuedToDraft, setComposerDraft, useComposerDraftsStore } from "./composerDrafts";
import type { QueuedMessage } from "./sessionStore";

const queued = (text: string): QueuedMessage => ({ text, attachments: [], promptName: null });

beforeEach(() => useComposerDraftsStore.setState({ byWs: {} }));

describe("returnQueuedToDraft", () => {
  it("puts the messages a stop took back ahead of the unsent text", () => {
    setComposerDraft("ws", 1, "still typing");
    returnQueuedToDraft("ws", 1, [queued("first"), queued("second ")]);
    expect(composerDraft("ws", 1)).toBe("first\n\nsecond\n\nstill typing");
  });

  it("fills an empty composer and touches no other tab", () => {
    setComposerDraft("ws", 2, "other tab");
    returnQueuedToDraft("ws", 1, [queued("only")]);
    expect(composerDraft("ws", 1)).toBe("only");
    expect(composerDraft("ws", 2)).toBe("other tab");
  });
});
