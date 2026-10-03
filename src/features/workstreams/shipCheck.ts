import { create } from "zustand";

export type ShipCheckTarget = { workstreamId: string; projectPath: string };

// Every Ship entry point opens the readiness dialog first; `shipping` is shared with the dialog.
export const useShipCheck = create<{ target: ShipCheckTarget | null; shipping: boolean }>(() => ({
  target: null,
  shipping: false,
}));

export function openShipCheck(target: ShipCheckTarget): void {
  useShipCheck.setState({ target });
}

export function closeShipCheck(): void {
  useShipCheck.setState({ target: null });
}

export function setShipping(shipping: boolean): void {
  useShipCheck.setState({ shipping });
}
