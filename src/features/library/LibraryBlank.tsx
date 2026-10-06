import { useNavigate } from "@tanstack/react-router";

import { BLANK_CTA, BLANK_CTA_SECONDARY, BlankState } from "@/components/BlankState";
import { Icon } from "@/components/Icon";
import { Button, Kbd } from "@/components/ui";
import type { LaunchStage } from "@/lib/domain/stages";
import { useChord } from "@/lib/keybindings/useChord";

const PROMPT_BUTTONS: { stage: LaunchStage; command: string }[] = [
  { stage: "research", command: "/research_codebase" },
  { stage: "plan", command: "/create_plan" },
];

export function LibraryBlank({
  thoughtsConfigured,
  targetProject,
  onNewNote,
}: {
  thoughtsConfigured: boolean;
  targetProject: string | undefined;
  onNewNote: () => void;
}) {
  const navigate = useNavigate();
  const newChord = useChord("doc.new");

  return (
    <BlankState
      art="library"
      align="center"
      title={
        <>
          An empty
          <br />
          shelf.
        </>
      }
      lede="Research, plans, handoffs and notes land here as markdown the moment an agent writes one."
      actions={
        thoughtsConfigured ? (
          <>
            <Button variant="primary" className={BLANK_CTA} onClick={onNewNote}>
              <Icon name="doc" size="sm" />
              New thought
            </Button>
            {targetProject &&
              PROMPT_BUTTONS.map(({ stage, command }) => (
                <Button
                  key={stage}
                  className={BLANK_CTA_SECONDARY}
                  onClick={() =>
                    void navigate({
                      to: "/projects/$projectPath",
                      params: { projectPath: targetProject },
                      search: { create: true, stage },
                    })
                  }
                >
                  <span className="font-mono text-ui-sm">{command}</span>
                </Button>
              ))}
          </>
        ) : (
          <Button variant="primary" className={BLANK_CTA} onClick={() => void navigate({ to: "/onboarding" })}>
            Set up thoughts
          </Button>
        )
      }
      hint={
        thoughtsConfigured && (
          <>
            {newChord && (
              <>
                <Kbd chord={newChord} /> new thought ·{" "}
              </>
            )}
            files live in thoughts/ and are yours without the app
          </>
        )
      }
    />
  );
}
