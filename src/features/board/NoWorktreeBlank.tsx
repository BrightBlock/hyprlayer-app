import { useNavigate } from "@tanstack/react-router";

import { BLANK_CTA, BLANK_CTA_SECONDARY, BlankState } from "@/components/BlankState";
import { Icon } from "@/components/Icon";
import { Button, Kbd } from "@/components/ui";
import { plural } from "@/lib/format";
import { useChord } from "@/lib/keybindings/useChord";
import { useArchivedWorkstreams } from "@/lib/queries";

export function NoWorktreeBlank({
  projectPath,
  folder,
  onNew,
  onRestore,
}: {
  projectPath: string;
  folder: boolean;
  onNew: () => void;
  onRestore: () => void;
}) {
  const navigate = useNavigate();
  const newChord = useChord("ws.new");
  const archived = useArchivedWorkstreams();
  const archivedCount = (archived.data ?? []).filter((v) => v.item.projectPath === projectPath).length;
  const archivedHint =
    !folder && archivedCount > 0 ? `${plural(archivedCount, "archived worktree")} can be restored` : null;

  return (
    <BlankState
      art="session"
      align="center"
      title={
        folder ? (
          <>
            Start the
            <br />
            first <em className="italic">session.</em>
          </>
        ) : (
          <>
            Start the
            <br />
            first <em className="italic">branch.</em>
          </>
        )
      }
      lede={
        folder
          ? "Every session is its own conversation, all running in this folder. Start as many as you need."
          : "Every piece of work gets its own worktree, so two agents never fight over the same files. Name the work and Hyprlayer cuts the branch, opens the chat and starts the terminal."
      }
      actions={
        <>
          <Button variant="primary" className={BLANK_CTA} onClick={onNew}>
            {!folder && <Icon name="branch" size="sm" />}
            {folder ? "New session" : "New worktree"}
          </Button>
          <Button className={BLANK_CTA_SECONDARY} onClick={() => void navigate({ to: "/pipeline" })}>
            See the pipeline
          </Button>
          {archivedCount > 0 && (
            <Button className={BLANK_CTA_SECONDARY} onClick={onRestore}>
              Restore an archived one
            </Button>
          )}
        </>
      }
      hint={
        (newChord || archivedHint) && (
          <span>
            {newChord && (
              <>
                <Kbd chord={newChord} /> {folder ? "new session" : "new worktree"}
              </>
            )}
            {newChord && archivedHint && " · "}
            {archivedHint}
          </span>
        )
      }
    />
  );
}
