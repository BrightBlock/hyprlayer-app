import { IS_MACOS } from "@/lib/platform";
import { useBootstrap } from "@/lib/queries";
import type { Chord } from "./chord";
import { effectiveChord } from "./resolve";

export function useChord(id: string): Chord | null {
  const bootstrap = useBootstrap();
  return effectiveChord(bootstrap.data?.settings.keybindings?.overrides ?? {}, id, IS_MACOS);
}
