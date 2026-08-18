/** The composer grows from one row up to this height, then scrolls. ~14 rows of
 *  text-sm — generous enough to keep a whole prompt in view without letting the
 *  box crowd out the transcript above it. */
export const MAX_COMPOSER_HEIGHT = 320;

/** The slice of a textarea this measurement touches, so the sizing rule can be
 *  tested without a layout engine. */
export type SizableTextarea = {
  style: { height: string; overflowY: string };
  readonly scrollHeight: number;
};

/** Size a textarea to its content, capped at `max`, and return the height used.
 *
 *  The scrollbar is pinned off for the measurement and stays off until the box
 *  is actually capped. A vertical scrollbar takes real layout width on
 *  Linux/WebKitGTK (macOS overlays it), so a bar showing mid-measurement wraps
 *  the text against a narrower line than the final box will have: we land a row
 *  too tall, the extra row means the content now fits, the bar disappears, the
 *  text re-wraps wider — and any observer watching the width measures again.
 *  That loop is what flickered at the wrap point. Below the cap there is
 *  nothing to scroll anyway, so `hidden` costs nothing and breaks the cycle. */
export function fitComposerHeight(
  ta: SizableTextarea,
  max: number = MAX_COMPOSER_HEIGHT,
): number {
  ta.style.overflowY = "hidden";
  // `auto` first (one row, per the `rows` attribute) so the box can shrink as
  // well as grow — scrollHeight then reports what the content actually needs.
  ta.style.height = "auto";
  const fit = Math.min(ta.scrollHeight, max);
  ta.style.height = `${fit}px`;
  ta.style.overflowY = fit < max ? "hidden" : "auto";
  return fit;
}
