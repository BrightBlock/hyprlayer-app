import { harnessIcon } from "@/components/icons";
import { cx, DropdownSelect } from "@/components/ui";
import { DEFAULT_HARNESS_OPTIONS } from "@/lib/domain/harnessing";
import { useRovingRadio } from "@/lib/useRovingRadio";

type HarnessPickerProps = {
  value: string;
  onChange: (key: string) => void;
  variant: "radio" | "dropdown";
  disabled?: boolean;
};

const OPTIONS = DEFAULT_HARNESS_OPTIONS.map((h) => ({ value: h.key, label: h.label }));

/** The default harness, as General's radio row or the popover's compact dropdown — one value. */
export function HarnessPicker({ variant, ...props }: HarnessPickerProps) {
  return variant === "radio" ? <HarnessRadio {...props} /> : <HarnessDropdown {...props} />;
}

function HarnessRadio({ value, onChange, disabled }: Omit<HarnessPickerProps, "variant">) {
  const { groupProps, itemProps } = useRovingRadio(OPTIONS, value, onChange, disabled);
  return (
    <div {...groupProps} aria-label="Default harness" className="grid grid-cols-4 gap-2">
      {DEFAULT_HARNESS_OPTIONS.map((h, i) => {
        const Icon = harnessIcon(h.key);
        const selected = h.key === value;
        return (
          <button
            key={h.key}
            type="button"
            {...itemProps(i)}
            className={cx(
              "flex h-[38px] min-w-0 items-center justify-center gap-2 rounded-control border bg-panel px-2 text-[13px] transition-colors",
              "focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
              "disabled:cursor-not-allowed disabled:opacity-50",
              selected
                ? "border-ink font-bold text-ink ring-1 ring-inset ring-ink"
                : "border-edge text-ink-2 enabled:hover:bg-panel-2 enabled:hover:text-ink",
            )}
          >
            <Icon size={16} />
            <span className="truncate">{h.label}</span>
          </button>
        );
      })}
    </div>
  );
}

function HarnessDropdown({ value, onChange, disabled }: Omit<HarnessPickerProps, "variant">) {
  return (
    <DropdownSelect
      aria-label="Default harness"
      value={value}
      options={OPTIONS}
      onChange={onChange}
      disabled={disabled}
      className="h-6 w-auto gap-1.5 bg-panel px-2 py-0 text-xs"
    />
  );
}
