import type { ErrorComponentProps } from "@tanstack/react-router";

import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";

import { Button, ErrorNote } from "./ui";

/**
 * A page that threw while rendering. Every route gets its own boundary, so the
 * rail and window controls around it keep working.
 */
export function RouteError({ error, reset }: ErrorComponentProps) {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="flex w-full max-w-lg flex-col gap-3">
        <ErrorNote error={error} />
        <div className="flex gap-2">
          <Button onClick={reset}>Try again</Button>
          <Button
            onClick={() =>
              void unwrap(commands.revealLogs()).catch((e) =>
                log.warn("route-error", "reveal logs failed", e),
              )
            }
          >
            Reveal logs
          </Button>
        </div>
      </div>
    </div>
  );
}
