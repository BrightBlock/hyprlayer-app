import { type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_LINUX, IS_MACOS } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";

export function WindowControls() {
  const maximized = useIsMaximized();

  if (IS_MACOS) return null;

  const win = getCurrentWindow();
  return (
    <div className="flex h-full shrink-0 items-stretch">
      {!IS_LINUX && (
        <>
          <CaptionButton label="Minimize" onClick={() => void win.minimize()}>
            <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
              <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
            </svg>
          </CaptionButton>
          <CaptionButton
            label={maximized ? "Restore" : "Maximize"}
            onClick={() => void win.toggleMaximize()}
          >
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                <rect x="0.5" y="2.5" width="7" height="7" stroke="currentColor" strokeWidth="1" />
                <path d="M2.5 2.5V0.5h7v7h-2" stroke="currentColor" strokeWidth="1" />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" strokeWidth="1" />
              </svg>
            )}
          </CaptionButton>
        </>
      )}
      <CaptionButton label="Close" onClick={() => void win.close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </CaptionButton>
    </div>
  );
}

function CaptionButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className="group inline-flex h-full w-9 items-center justify-center text-ink-dim"
    >
      <span className="flex h-7 w-7 translate-y-px items-center justify-center rounded-full transition-colors group-hover:bg-edge/60 group-hover:text-ink">
        {children}
      </span>
    </button>
  );
}
