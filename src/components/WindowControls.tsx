import { type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { IS_LINUX, IS_MACOS } from "@/lib/platform";
import { useIsMaximized } from "@/lib/useMaximized";

/**
 * Caption buttons drawn into the app header. macOS keeps its
 * native traffic lights overlaid on the header (titleBarStyle: "Overlay"), so
 * this renders only OFF macOS, where the native title bar is removed
 * (`decorations: false`, see tauri.windows.conf.json / tauri.linux.conf.json)
 * and the app must supply its own controls. That removal is what kills the
 * second top bar Windows used to draw above the header.
 *
 * Windows gets the full minimize / maximize-restore / close set. Linux gets
 * Close alone, matching Brave and the GNOME convention there: minimize and
 * maximize are the compositor's job, still reachable through the WM (keybind,
 * double-click on the drag region, super-drag) and meaningless on a tiling WM,
 * where a client-drawn maximize button does nothing legible.
 */
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
        {/* 10px, edge-to-edge in its own viewBox: the header's other glyphs are
            inset in a 24 box, so they need a larger render size to land at this
            same ~10px visible height. Sized down there, not up here. */}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </CaptionButton>
    </div>
  );
}

/**
 * w-9 (not the usual caption-button width) so the glyph centre lands 36px from
 * the header's last icon button, the same pitch the bar's own controls sit at.
 *
 * The hover state is a filled circle behind the glyph rather than a full-height
 * bar, and Close is grey like the rest, no Windows-red wash. That is the
 * GNOME/Brave treatment, and it is what reads correctly next to a lone Close
 * button on Linux, where a full-bleed red block in the corner is the loudest
 * thing in the header.
 */
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
      {/* translate-y-px: measured against a screenshot, the caption glyph's ink
          centre landed ~0.7px above the bar's other icons even though both are
          flex-centred in the same 39px band, so it read high next to the
          sidebar toggle. The circle rides down with the glyph to stay
          concentric with it. */}
      <span className="flex h-7 w-7 translate-y-px items-center justify-center rounded-full transition-colors group-hover:bg-edge/60 group-hover:text-ink">
        {children}
      </span>
    </button>
  );
}
