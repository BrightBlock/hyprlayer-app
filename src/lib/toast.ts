import { create } from "zustand";

export type ToastTone = "ok" | "warn" | "bad";

export interface ToastAction {
  label: string;
  /** Shown (disabled) while the action is running, if it returns a promise. */
  busyLabel?: string;
  onClick: () => void | Promise<void>;
}

interface ShowOptions {
  tone?: ToastTone;
  /** Optional primary action button. A toast with an action is sticky — it
   *  stays until the user acts on or dismisses it (no countdown auto-dismiss). */
  action?: ToastAction;
  /** Fired when the user dismisses the toast (×). Lets the update toast mark
   *  its version dismissed so a later background check won't re-nag. */
  onDismiss?: () => void;
}

interface ToastState {
  visible: boolean;
  title: string;
  message: string;
  tone: ToastTone;
  action: ToastAction | null;
  onDismiss: (() => void) | null;
  show: (title: string, message: string, opts?: ShowOptions) => void;
  hide: () => void;
}

export const useToast = create<ToastState>((set, get) => ({
  visible: false,
  title: "",
  message: "",
  tone: "ok",
  action: null,
  onDismiss: null,
  show: (title, message, opts) =>
    set({
      visible: true,
      title,
      message,
      tone: opts?.tone ?? "ok",
      action: opts?.action ?? null,
      onDismiss: opts?.onDismiss ?? null,
    }),
  hide: () => {
    const cb = get().onDismiss;
    set({ visible: false, onDismiss: null });
    cb?.();
  },
}));
