import { create } from "zustand";

export type ToastTone = "ok" | "warn" | "bad";

interface ToastState {
  visible: boolean;
  title: string;
  message: string;
  tone: ToastTone;
  show: (title: string, message: string, tone?: ToastTone) => void;
  hide: () => void;
}

export const useToast = create<ToastState>((set) => ({
  visible: false,
  title: "",
  message: "",
  tone: "ok",
  show: (title, message, tone = "ok") =>
    set({ visible: true, title, message, tone }),
  hide: () => set({ visible: false }),
}));
