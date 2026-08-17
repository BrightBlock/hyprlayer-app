import { type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_MACOS } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";

/**
 * VSCode-style caption buttons (minimize / maximize-restore / close) drawn into
 * the app header. macOS keeps its native traffic lights overlaid on the header
 * (titleBarStyle: "Overlay"), so this renders only OFF macOS — where the native
 * title bar is removed (`decorations: false`, see tauri.windows.conf.json /
 * tauri.linux.conf.json) and the app must supply its own controls. That removal
 * is what kills the second top bar Windows used to draw above the header.
 */
export function WindowControls() {
  const maximized = useIsMaximized();

  if (IS_MACOS) return null;

  const win = getCurrentWindow();
  return (
    <div className="flex h-full shrink-0 items-stretch">
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
      <CaptionButton label="Close" close onClick={() => void win.close()}>
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
  close,
  children,
}: {
  label: string;
  onClick: () => void;
  close?: boolean;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      className={`inline-flex h-full w-12 items-center justify-center text-ink-dim transition-colors ${
        close
          ? "hover:bg-[#e81123] hover:text-white"
          : "hover:bg-edge/60 hover:text-ink"
      }`}
    >
      {children}
    </button>
  );
}
