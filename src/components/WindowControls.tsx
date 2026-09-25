import { type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_LINUX, IS_MACOS } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";
import { Icon } from "./Icon";

export function WindowControls() {
  const maximized = useIsMaximized();

  if (IS_MACOS) return null;

  const win = getCurrentWindow();
  return (
    <div className="flex h-full shrink-0 items-stretch">
      {!IS_LINUX && (
        <>
          <CaptionButton label="Minimize" onClick={() => void win.minimize()}>
            <Icon name="windowMinimize" size="xs" />
          </CaptionButton>
          <CaptionButton
            label={maximized ? "Restore" : "Maximize"}
            onClick={() => void win.toggleMaximize()}
          >
            <Icon name={maximized ? "windowRestore" : "windowMaximize"} size="xs" />
          </CaptionButton>
        </>
      )}
      <CaptionButton label="Close" onClick={() => void win.close()}>
        <Icon name="close" size="xs" />
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
