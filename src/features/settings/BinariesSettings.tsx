import type { BinaryStatus } from "@/lib/bindings";
import { useRedetectBinaries } from "@/lib/queries";
import { Button, Card, CardTitle } from "@/components/ui";
import { BinaryCard } from "./BinaryCard";

export function BinariesSettings({ binaries }: { binaries: BinaryStatus[] }) {
  const redetect = useRedetectBinaries();
  return (
    <Card>
      <div className="flex items-center justify-between">
        <CardTitle className="mb-0">Binaries</CardTitle>
        <Button onClick={() => redetect.mutate()} busy={redetect.isPending}>
          Re-detect
        </Button>
      </div>
      <p className="mt-1 text-xs text-ink-dim">
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
