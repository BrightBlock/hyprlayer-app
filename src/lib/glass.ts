import { IS_MACOS } from "./platform";

export const GLASS_BAND = IS_MACOS
  ? "bg-panel/55 backdrop-blur-md backdrop-saturate-150 backdrop-brightness-125"
  : "bg-panel";
