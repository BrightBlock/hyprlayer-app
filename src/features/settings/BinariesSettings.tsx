import type { BinaryStatus } from "@/lib/bindings";
import { useRedetectBinaries } from "@/lib/queries";
import { SETTINGS_CARD_DESCRIPTION, SettingsCardTitle } from "@/components/SettingsCard";
import { Button, Card, cx } from "@/components/ui";
import { BinaryCard } from "./BinaryCard";

export function BinariesSettings({ binaries }: { binaries: BinaryStatus[] }) {
  const redetect = useRedetectBinaries();
  return (
    <Card>
      <div className="flex items-center justify-between">
        <SettingsCardTitle className="mb-0">Binaries</SettingsCardTitle>
        <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
          Re-detect
        </Button>
      </div>
      <p className={cx("mt-1", SETTINGS_CARD_DESCRIPTION)}>
        The app searches your shell PATH plus common install locations. Set an explicit path if a
        tool isn't found.
      </p>
      <div className="mt-3 space-y-2">
        {binaries.map((b) => (
          <BinaryCard key={b.kind} status={b} />
        ))}
      </div>
    </Card>
  );
}
