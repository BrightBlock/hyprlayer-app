import { useEffect, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";

import type { ReviewSide } from "@/lib/bindings";

import { type CommentLayer, slotKey } from "./commentGutter";

export type Slot = { side: ReviewSide; line: number; content: ReactNode };

type ComposerAt = { side: ReviewSide; line: number };

const SIDES: readonly ReviewSide[] = ["original", "modified"];

export function ThreadPortals({
  layer,
  slots,
  composer,
}: {
  layer: CommentLayer;
  slots: Slot[];
  composer: ComposerAt | null;
}) {
  const version = useSyncExternalStore(layer.subscribe, layer.getVersion);

  const shape = slots.map((s) => slotKey(s.side, s.line)).join(",");
  const composerKey = composer ? slotKey(composer.side, composer.line) : "";
  for (const side of SIDES) {
    const lines = [...new Set(slots.filter((s) => s.side === side).map((s) => s.line))].sort((a, b) => a - b);
    layer.want(side, { lines, composer: composer?.side === side ? composer.line : null });
  }

  useEffect(() => {
    layer.flush();
  }, [layer, version, shape, composerKey]);

  return slots.map((s) => {
    const view = layer.view(s.side);
    if (!view || s.line < 1 || s.line > view.state.doc.lines) return null;
    const key = slotKey(s.side, s.line);
    return createPortal(s.content, layer.container(s.side, s.line), key);
  });
}
