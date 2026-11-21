import { openUrl } from "@tauri-apps/plugin-opener";
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { log } from "./log";

type BrowserStore = {
  browser: string | null;
  setBrowser: (browser: string | null) => void;
};

export const useBrowser = create<BrowserStore>()(
  persist((set) => ({ browser: null, setBrowser: (browser) => set({ browser }) }), {
    name: "hyprlayer:browser",
  }),
);

export async function openExternalUrl(url: string): Promise<void> {
  const { browser } = useBrowser.getState();
  if (browser) {
    try {
      return await openUrl(url, browser);
    } catch (e) {
      log.warn("browser", `opening a link in ${browser} failed, using the system default`, e);
    }
  }
  await openUrl(url);
}
