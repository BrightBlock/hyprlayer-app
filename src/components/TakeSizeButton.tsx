import { commands } from "@/lib/bindings";
import { Icon } from "./Icon";
import { Button } from "./ui";

export function TakeSizeButton({ terminalId }: { terminalId: () => string | undefined }) {
  return (
    <Button
      size="sm"
      className="absolute right-3 top-3 z-10 shadow-pop"
      onClick={() => {
        const id = terminalId();
        if (id) void commands.terminalTakeSize(id);
      }}
    >
      <Icon name="takeSize" size="xs" />
      Use this window's size
    </Button>
  );
}
