import { useEffect, useRef, useState } from "react";

import { useToast, type ToastTone } from "@/lib/toast";
import { XIcon } from "./icons";

const DURATION_MS = 5000;
const EXIT_MS = 380; // must match toast-out animation duration

function ToastIcon({ tone }: { tone: ToastTone }) {
  const base = "flex h-6 w-6 shrink-0 items-center justify-center rounded-full";
  if (tone === "ok") {
    return (
      <span className={`${base} bg-ok`}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path
            d="M5 12l5 5 9-9"
            stroke="var(--color-surface)"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </span>
    );
  }
  if (tone === "warn") {
    return (
      <span className={`${base} bg-warn`}>
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M12 8v5M12 16h.01" stroke="var(--color-surface)" strokeWidth="2.5" strokeLinecap="round" />
        </svg>
      </span>
    );
  }
  return (
    <span className={`${base} bg-bad`}>
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path
          d="M6 6l12 12M18 6L6 18"
          stroke="var(--color-surface)"
          strokeWidth="2.5"
          strokeLinecap="round"
        />
      </svg>
    </span>
  );
}

export function Toast() {
  const { visible, title, message, tone, action, hide } = useToast();
  const dismissRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tickRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const unmountRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startRef = useRef<number>(0);

  const [remaining, setRemaining] = useState(Math.ceil(DURATION_MS / 1000));
  const [mounted, setMounted] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [acting, setActing] = useState(false);

  // A toast with an action sticks around, the user has something to do, so we
  // don't yank it out from under them on a countdown.
  const sticky = action !== null;

  // Mount/unmount with exit animation
  useEffect(() => {
    if (visible) {
      if (unmountRef.current) clearTimeout(unmountRef.current);
      setExiting(false);
      setMounted(true);
    } else {
      setExiting(true);
      unmountRef.current = setTimeout(() => {
        setMounted(false);
        setExiting(false);
      }, EXIT_MS);
    }
    return () => { if (unmountRef.current) clearTimeout(unmountRef.current); };
  }, [visible]);

  // Countdown + auto-dismiss, skipped for sticky (action) toasts.
  useEffect(() => {
    if (!visible || sticky) return;

    setRemaining(Math.ceil(DURATION_MS / 1000));
    startRef.current = performance.now();

    tickRef.current = setInterval(() => {
      const elapsed = performance.now() - startRef.current;
      setRemaining(Math.max(0, Math.ceil((DURATION_MS - elapsed) / 1000)));
    }, 200);

    dismissRef.current = setTimeout(() => {
      if (tickRef.current) clearInterval(tickRef.current);
      hide();
    }, DURATION_MS);

    return () => {
      if (dismissRef.current) clearTimeout(dismissRef.current);
      if (tickRef.current) clearInterval(tickRef.current);
    };
  }, [visible, sticky, hide]);

  if (!mounted) return null;

  const runAction = async () => {
    if (!action || acting) return;
    setActing(true);
    try {
      await action.onClick();
    } finally {
      setActing(false);
    }
  };

  return (
    <div
      role="status"
      aria-live="polite"
      className={`fixed bottom-6 right-6 z-50 flex items-start ${exiting ? "animate-toast-out pointer-events-none" : "animate-toast-in"}`}
    >
      <button
        type="button"
        onClick={hide}
        aria-label="Dismiss"
        className="-mr-4 z-10 mt-2 flex h-6 shrink-0 items-center gap-1 rounded-full border border-edge bg-panel-2 px-2 text-ink-dim hover:text-ink"
      >
        <XIcon size={9} />
        {!sticky && <span className="text-2xs tabular-nums leading-none">{remaining}s</span>}
      </button>
      <div className="flex min-w-72 max-w-sm items-start gap-3 rounded-lg border border-edge bg-panel px-4 py-3.5 shadow-2xl">
        <ToastIcon tone={tone} />
        <div className="min-w-0">
          <p className="text-sm font-semibold text-ink">{title}</p>
          <p className="mt-0.5 text-xs text-ink-dim">{message}</p>
          {action && (
            <button
              type="button"
              onClick={() => void runAction()}
              disabled={acting}
              className="mt-2.5 inline-flex items-center rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-accent-ink hover:bg-accent/90 disabled:opacity-60"
            >
              {acting ? (action.busyLabel ?? action.label) : action.label}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
