import { cx } from "@/components/ui";
import { meterClasses, meterState } from "./statusBarState";

export function UsageMeter({
  label,
  percent,
  title,
}: {
  label: string;
  percent: number;
  title: string;
}) {
  const { fill, text } = meterClasses(meterState(percent));
  return (
    <div
      role="meter"
      aria-label={title}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      title={title}
      className="flex shrink-0 items-center gap-1.5"
    >
      <span className="text-ui-sm text-ink-4">{label}</span>
      <span className="h-1 w-[34px] overflow-hidden rounded-full bg-edge">
        <span className={cx("block h-full rounded-full", fill)} style={{ width: `${percent}%` }} />
      </span>
      <span className={cx("text-ui-sm", text)}>{percent}%</span>
    </div>
  );
}
