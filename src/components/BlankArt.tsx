import firstRun from "@/assets/art/first-run.svg?url";
import session from "@/assets/art/session.svg?url";
import library from "@/assets/art/library.svg?url";

const ART = { firstRun, session, library } as const;
export type BlankArtName = keyof typeof ART;

// Mixing toward the surface keeps stroke contrast proportional on every theme.
const ART_INK = "color-mix(in srgb, var(--color-ink) 88%, var(--color-surface))";
const stop = (pct: number, at: string) => `color-mix(in srgb, var(--color-surface) ${pct}%, transparent) ${at}`;
const FADE = {
  wizard: `linear-gradient(90deg, var(--color-surface) 30%, ${stop(80, "50%")}, transparent 76%)`,
  pane: `linear-gradient(90deg, var(--color-surface) 12%, ${stop(72, "46%")}, transparent 78%)`,
} as const;

export function BlankArt({ art, fade, mirror = false }: { art: BlankArtName; fade: keyof typeof FADE; mirror?: boolean }) {
  const mask = `url(${ART[art]})`;
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 overflow-hidden">
      <div
        className="absolute inset-x-0 bottom-0 aspect-[1440/940]"
        style={{
          backgroundColor: ART_INK,
          maskImage: mask,
          WebkitMaskImage: mask,
          maskSize: "100% 100%",
          WebkitMaskSize: "100% 100%",
          maskRepeat: "no-repeat",
          WebkitMaskRepeat: "no-repeat",
          transform: mirror ? "scaleX(-1)" : undefined,
        }}
      />
      <div className="absolute inset-0" style={{ background: FADE[fade] }} />
    </div>
  );
}
