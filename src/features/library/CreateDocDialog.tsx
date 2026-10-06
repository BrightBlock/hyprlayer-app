import { useState, type FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";

import { Button, Dialog, DropdownSelect, ErrorNote, Field, TextInput } from "@/components/ui";
import { DOC_SCOPES, DOC_TYPES } from "@/lib/domain/docs";
import { isGitProject } from "@/lib/domain/projects";
import { useBootstrap, useCreateDoc } from "@/lib/queries";
import { parseTagsInput } from "./text";

type Props = {
  open: boolean;
  onClose: () => void;
  initialProjectName?: string;
};

export function CreateDocDialog({ open, onClose, initialProjectName }: Props) {
  if (!open) return null;
  return <CreateDocForm onClose={onClose} initialProjectName={initialProjectName} />;
}

function CreateDocForm({ onClose, initialProjectName }: Omit<Props, "open">) {
  const bootstrap = useBootstrap();
  const createDoc = useCreateDoc();
  const navigate = useNavigate();

  const projects = (bootstrap.data?.projects ?? []).filter(isGitProject);

  const [title, setTitle] = useState("");
  const [docType, setDocType] = useState<string>("note");
  const [scope, setScope] = useState<string>("user");
  const [projectPath, setProjectPath] = useState(
    () =>
      projects.find((p) => p.thoughtsProject === initialProjectName)?.path ?? projects[0]?.path ?? "",
  );
  const [ticket, setTicket] = useState("");
  const [tags, setTags] = useState("");

  const isGlobal = scope === "global";
  const needsProject = !isGlobal && projectPath === "";
  const canSubmit = title.trim().length > 0 && !needsProject && !createDoc.isPending;

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    createDoc.mutate(
      {
        projectRepoPath: isGlobal ? null : projectPath,
        scope,
        docType,
        title: title.trim(),
        date: new Date().toISOString().slice(0, 10),
        ticket: ticket.trim() || null,
        tags: parseTagsInput(tags),
      },
      {
        onSuccess: (meta) => {
          onClose();
          void navigate({ to: "/library", search: { doc: meta.id } });
        },
      },
    );
  }

  return (
    <Dialog open onClose={onClose} title="New thought">
      <form onSubmit={submit} className="space-y-3">
        <Field label="Title">
          <TextInput
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            autoFocus
            required
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Type">
            <DropdownSelect
              aria-label="Type"
              value={docType}
              options={DOC_TYPES.map((t) => ({ value: t, label: t }))}
              onChange={setDocType}
            />
          </Field>
          <Field label="Scope">
            <DropdownSelect
              aria-label="Scope"
              value={scope}
              options={DOC_SCOPES.map((s) => ({ value: s, label: s }))}
              onChange={setScope}
            />
          </Field>
        </div>
        {!isGlobal && (
          <Field label="Project">
            <DropdownSelect
              aria-label="Project"
              value={projectPath}
              options={
                projects.length === 0
                  ? [{ value: "", label: "No projects mapped" }]
                  : projects.map((p) => ({ value: p.path, label: p.mappedName }))
              }
              onChange={setProjectPath}
            />
            {projects.length === 0 && (
              <p className="mt-1 text-ui text-warn-ink">
                Add a project first, or pick the global scope.
              </p>
            )}
          </Field>
        )}
        <Field label="Ticket">
          <TextInput
            value={ticket}
            onChange={(e) => setTicket(e.target.value)}
            placeholder="ENG-1234"
          />
        </Field>
        <Field label="Tags">
          <TextInput
            value={tags}
            onChange={(e) => setTags(e.target.value)}
            placeholder="refactor, ipc, phase-1"
          />
        </Field>
        {createDoc.isError && (
          <ErrorNote error={createDoc.error} onDismiss={() => createDoc.reset()} />
        )}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={!canSubmit} busy={createDoc.isPending}>
            Create
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
