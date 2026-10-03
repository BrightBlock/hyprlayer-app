import { useEffect, useRef, useState } from "react";

import { STATUS_BAR_HEIGHT } from "@/lib/layout";
import { useToast, type ToastTone } from "@/lib/toast";
import { Icon, type IconName } from "./Icon";

const DURATION_MS = 5000;
const EXIT_MS = 380;

const TONE_BADGE: Record<ToastTone, { name: IconName; bg: string }> = {
  ok: { name: "check", bg: "bg-ok" },
  warn: { name: "warning", bg: "bg-warn" },
  bad: { name: "close", bg: "bg-bad" },
};

function ToastIcon({ tone }: { tone: ToastTone }) {
  const badge = TONE_BADGE[tone];
  return (
    <span className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full ${badge.bg}`}>
      <Icon name={badge.name} size="xs" className="text-surface" />
    </span>
  );
}

export function Toast() {
  const { visible, title, message, tone, action, timed, hide } = useToast();
  const dismissRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const tickRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const unmountRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const startRef = useRef<number>(0);

  const [remaining, setRemaining] = useState(Math.ceil(DURATION_MS / 1000));
  const [mounted, setMounted] = useState(false);
  const [exiting, setExiting] = useState(false);
  const [acting, setActing] = useState(false);

  const sticky = action !== null && !timed;

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
      className={`fixed right-6 z-50 flex items-start ${exiting ? "animate-toast-out pointer-events-none" : "animate-toast-in"}`}
      style={{ bottom: STATUS_BAR_HEIGHT + 24 }}
    >
      <button
        type="button"
        onClick={hide}
        aria-label="Dismiss"
        className="-mr-4 z-10 mt-2 flex h-6 shrink-0 items-center gap-1 rounded-full border border-edge bg-panel-2 px-2 text-ink-dim hover:text-ink"
      >
        <Icon name="close" size="xs" />
        {!sticky && <span className="text-ui-sm tabular-nums leading-none">{remaining}s</span>}
      </button>
      <div className="flex min-w-72 max-w-sm items-start gap-3 rounded-lg border border-edge bg-panel px-4 py-3.5 shadow-2xl">
        <ToastIcon tone={tone} />
        <div className="min-w-0">
          <p className="text-body font-semibold text-ink">{title}</p>
          <p className="mt-0.5 text-ui text-ink-dim">{message}</p>
          {action && (
            <button
              type="button"
              onClick={() => void runAction()}
              disabled={acting}
              className="mt-2.5 inline-flex items-center rounded-md bg-accent px-2.5 py-1 text-ui font-medium text-accent-ink hover:bg-accent/90 disabled:opacity-60"
            >
              {acting ? (action.busyLabel ?? action.label) : action.label}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
