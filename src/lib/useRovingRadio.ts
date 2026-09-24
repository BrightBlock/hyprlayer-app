import { useRef, type KeyboardEvent } from "react";

export type RovingOption<T extends string> = { value: T; disabled?: boolean };

const STEP: Record<string, number> = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };

/** APG radio group: one tab stop; arrows move and select, wrapping past disabled options. */
export function useRovingRadio<T extends string>(
  options: ReadonlyArray<RovingOption<T>>,
  value: T | null,
  onChange: (value: T) => void,
  disabled = false,
) {
  const refs = useRef<Array<HTMLElement | null>>([]);
  const enabled = options.map((o) => !disabled && !o.disabled);
  const selected = options.findIndex((o) => o.value === value);
  const stop = selected >= 0 && enabled[selected] ? selected : enabled.indexOf(true);

  const target = (from: number, key: string): number | null => {
    const n = options.length;
    if (key === "Home") return enabled.indexOf(true);
    if (key === "End") return enabled.lastIndexOf(true);
    const step = STEP[key];
    if (!step) return null;
    for (let k = 1; k <= n; k++) {
      const i = (((from + step * k) % n) + n) % n;
      if (enabled[i]) return i;
    }
    return null;
  };

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    const from = refs.current.findIndex((el) => el === document.activeElement);
    const next = target(from < 0 ? stop : from, e.key);
    if (next === null || next < 0) return;
    e.preventDefault();
    refs.current[next]?.focus();
    onChange(options[next].value);
  };

  const itemProps = (index: number) => ({
    ref: (el: HTMLElement | null) => {
      refs.current[index] = el;
    },
    role: "radio" as const,
    "aria-checked": index === selected,
    tabIndex: index === stop ? 0 : -1,
    disabled: !enabled[index],
    onClick: () => {
      if (enabled[index]) onChange(options[index].value);
    },
  });

  return { groupProps: { role: "radiogroup" as const, onKeyDown }, itemProps };
}
