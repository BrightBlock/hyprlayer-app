import { useNavigate } from "@tanstack/react-router";

import { BLANK_CTA, BLANK_CTA_SECONDARY, BlankState } from "@/components/BlankState";
import { Icon } from "@/components/Icon";
import { Button, Kbd } from "@/components/ui";
import { useChord } from "@/lib/keybindings/useChord";
import { useUpdate } from "@/lib/updater";

export function FirstRunBlank({
  onOpenProject,
  onClone,
  showFinishSetup,
}: {
  onOpenProject: () => void;
  onClone: () => void;
  showFinishSetup: boolean;
}) {
  const navigate = useNavigate();
  const version = useUpdate((s) => s.currentVersion);
  const openChord = useChord("project.open");
  const settingsChord = useChord("nav.settings");

  return (
    <BlankState
      art="firstRun"
      align="start"
      eyebrow={version ? `Hyprlayer ${version}` : undefined}
      title={
        <>
          Nothing
          <br />
          running yet.
        </>
      }
      lede="Point Hyprlayer at a repository and it opens a worktree, a chat and a terminal for every piece of work you start. Your plans, research and handoffs are written to files you can read without it."
      actions={
        <>
          <Button variant="primary" className={BLANK_CTA} onClick={onOpenProject}>
            <Icon name="folderOpen" size="sm" />
            Open a project
          </Button>
          <Button className={BLANK_CTA_SECONDARY} onClick={onClone}>
            <Icon name="branch" size="sm" />
            Clone a repository
          </Button>
          <Button
            className={BLANK_CTA_SECONDARY}
            onClick={() => void navigate({ to: "/settings", search: { section: "servers" } })}
          >
            <Icon name="server" size="sm" />
            Connect a server
          </Button>
        </>
      }
      hint={
        <span>
          {openChord && (
            <>
              <Kbd chord={openChord} /> open ·{" "}
            </>
          )}
          {settingsChord && (
            <>
              <Kbd chord={settingsChord} /> settings ·{" "}
            </>
          )}
          setup wizard runs on first open
        </span>
      }
      footer={
        showFinishSetup && (
          <button
            type="button"
            onClick={() => void navigate({ to: "/onboarding" })}
            className="mt-6 inline-flex items-center gap-1.5 text-ui text-ink-dim underline-offset-2 hover:text-ink hover:underline"
          >
            Finish setup
            <Icon name="arrowRight" size="xs" />
          </button>
        )
      }
    />
  );
}
