import type { ShellTab } from "./workstreamTabs";

export type OutgoingMessage = { sessionId: string; workstreamId: string; shell: ShellTab; text: string };

// Lets another feature consume a typed message instead of sending it (plan approval, wired in main.tsx).
let interceptor: ((message: OutgoingMessage) => Promise<boolean>) | null = null;

export function registerSendInterceptor(fn: (message: OutgoingMessage) => Promise<boolean>): void {
  interceptor = fn;
}

export function interceptSend(message: OutgoingMessage): Promise<boolean> {
  return interceptor ? interceptor(message) : Promise.resolve(false);
}
