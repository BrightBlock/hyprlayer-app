import { create } from "zustand";

export type ToastTone = "ok" | "warn" | "bad";

export interface ToastAction {
  label: string;
  busyLabel?: string;
  onClick: () => void | Promise<void>;
}

interface ShowOptions {
  tone?: ToastTone;
  action?: ToastAction;
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
