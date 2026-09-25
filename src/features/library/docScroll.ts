const MAX = 200;
const positions = new Map<string, number>();

export function rememberScroll(key: string, top: number) {
  positions.delete(key);
  positions.set(key, top);
  if (positions.size > MAX) positions.delete(positions.keys().next().value!);
}

export const recalledScroll = (key: string) => positions.get(key);
