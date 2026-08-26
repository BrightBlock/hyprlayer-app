export const MAX_COMPOSER_HEIGHT = 320;

export type SizableTextarea = {
  style: { height: string; overflowY: string };
  readonly scrollHeight: number;
};

export function fitComposerHeight(
  ta: SizableTextarea,
  max: number = MAX_COMPOSER_HEIGHT,
): number {
  ta.style.overflowY = "hidden";
  ta.style.height = "auto";
  const fit = Math.min(ta.scrollHeight, max);
  ta.style.height = `${fit}px`;
  ta.style.overflowY = fit < max ? "hidden" : "auto";
  return fit;
}
