import type { ReactNode } from "react";

import { BlankArt, type BlankArtName } from "@/components/BlankArt";
import { Icon } from "@/components/Icon";
import { HyprlayerMark } from "@/components/Logo";
import { cx } from "@/components/ui";
import { WindowControls } from "@/components/WindowControls";
import { dragRegion } from "@/lib/dragRegion";
import { useTitlebarInsetLights } from "@/lib/appearance";
import { TITLEBAR_INSET } from "@/lib/layout";
import { IS_MACOS } from "@/lib/platform";
import { useIsFullscreen } from "@/lib/useFullscreen";

type WizardStep = 1 | 2 | 3 | 4;

const WIZARD_STEPS = ["Your name", "Hyprlayer CLI", "AI agents", "Thoughts"];

const STEP_ART: Record<WizardStep, { art: BlankArtName; mirror: boolean }> = {
  1: { art: "firstRun", mirror: false },
  2: { art: "session", mirror: false },
  3: { art: "library", mirror: false },
  4: { art: "firstRun", mirror: true },
};

export function WizardFrame({
  step,
  onExit,
  children,
}: {
  step: WizardStep;
  onExit: () => void;
  children: ReactNode;
}) {
  const fullscreen = useIsFullscreen();
  const insetLights = useTitlebarInsetLights();
  const { art, mirror } = STEP_ART[step];

  return (
    <div className="flex h-full flex-col bg-surface">
      <header
        className="relative flex h-10 shrink-0 items-center border-b border-edge bg-panel"
        style={{ paddingLeft: IS_MACOS && !fullscreen ? insetLights : TITLEBAR_INSET }}
        {...dragRegion()}
      >
        <div className="flex items-center gap-2" {...dragRegion("deep")}>
          <HyprlayerMark size={20} />
          <span className="text-ui font-semibold text-ink">Hyprlayer</span>
          <span className="text-ui text-ink-4">· setup</span>
        </div>
        <ol
          {...dragRegion("deep")}
          aria-label="Setup progress"
          className="absolute left-1/2 flex -translate-x-1/2 gap-[22px] text-ui-sm"
        >
          {WIZARD_STEPS.map((label, i) => {
            const state = i + 1 < step ? "done" : i + 1 === step ? "current" : "future";
            return (
              <li
                key={label}
                aria-current={state === "current" ? "step" : undefined}
                className={cx(
                  "flex items-center gap-1",
                  state === "done" && "text-ink-dim",
                  state === "current" && "font-semibold text-ink",
                  state === "future" && "text-ink-4",
                )}
              >
                {state === "done" && <Icon name="check" size="xs" />}
                {label}
              </li>
            );
          })}
        </ol>
        <div className="ml-auto flex h-full items-center">
          <button
            type="button"
            onClick={onExit}
            className="px-3 text-ui text-ink-dim hover:text-ink"
          >
            Exit setup
          </button>
          <WindowControls />
        </div>
      </header>
      <div className="relative flex min-h-0 flex-1 overflow-y-auto bg-surface">
        <BlankArt art={art} mirror={mirror} fade="wizard" />
        <div key={step} className="relative z-[1] my-auto w-[760px] max-w-full animate-step-in px-24 py-10">
          {children}
        </div>
      </div>
    </div>
  );
}
