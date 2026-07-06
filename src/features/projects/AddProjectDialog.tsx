import { useEffect, useState } from "react";
import { homeDir, join } from "@tauri-apps/api/path";

import type { Project } from "@/lib/bindings";
import { Button, Dialog, ErrorNote, Field, TextInput } from "@/components/ui";
import {
  useAddProject,
  useBootstrap,
  useCloneProject,
  useDefaultUsername,
} from "@/lib/queries";
import { RepoSourcePicker, type RepoSource } from "./RepoSourcePicker";

type Props = {
  open: boolean;
  onClose: () => void;
  /** The freshly added project, so the caller can navigate to it. */
  onAdded: (project: Project) => void;
};

/**
 * Add a project by cloning a hosted repo into the repos root, or importing a
 * local repo from disk. A disk import is registered in place (never copied), so
 * worktrees branch off the user's existing repo, not a duplicate.
 */
export function AddProjectDialog({ open: isOpen, onClose, onAdded }: Props) {
  const bootstrap = useBootstrap();
  // A true first run has no config yet; the CLI needs a username to bootstrap one.
  const configMissing = bootstrap.data?.config.state === "missing";

  const [source, setSource] = useState<RepoSource>({ mode: "clone", url: "", location: "" });
  const [user, setUser] = useState("");

  const clone = useCloneProject();
  const add = useAddProject();
  const defaultUser = useDefaultUsername();

  const pending = clone.isPending || add.isPending;
  const error = clone.error ?? add.error;

  // On a first-run open, prefill "Your name" with the shell username ($USER) —
  // matching the CLI default. Keyed on open/data so it re-seeds each time the
  // dialog opens (after reset clears it) but never clobbers an in-session edit.
  useEffect(() => {
    if (isOpen && configMissing && defaultUser.data) {
      setUser((cur) => cur || defaultUser.data);
    }
  }, [isOpen, configMissing, defaultUser.data]);

  // Default the clone location to the configured repos root, else ~/hyprlayer/repos.
  useEffect(() => {
    if (!isOpen) return;
    const configured = bootstrap.data?.settings.reposRoot;
    if (configured) {
      setSource((cur) =>
        cur.mode === "clone" && !cur.location ? { ...cur, location: configured } : cur,
      );
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const def = await join(await homeDir(), "hyprlayer", "repos");
        if (!cancelled) {
          setSource((cur) =>
            cur.mode === "clone" && !cur.location ? { ...cur, location: def } : cur,
          );
        }
      } catch {
        // Leave blank → the backend falls back to its own default.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, bootstrap.data?.settings.reposRoot]);

  const reset = () => {
    setSource({ mode: "clone", url: "", location: "" });
    setUser("");
    clone.reset();
    add.reset();
  };

  const close = () => {
    if (pending) return;
    reset();
    onClose();
  };

  const succeed = (project: Project) => {
    reset();
    onClose();
    onAdded(project);
  };

  const bootstrapUser = configMissing ? user.trim() || null : null;

  const primaryInput =
    source.mode === "clone"
      ? source.url.trim()
      : source.mode === "disk"
        ? source.path.trim()
        : "";
  const canSubmit =
    Boolean(primaryInput) && (!configMissing || Boolean(user.trim())) && !pending;

  const submit = () => {
    if (!canSubmit) return;
    if (source.mode === "clone") {
      clone.mutate(
        {
          url: primaryInput,
          location: source.location.trim() || null,
          name: null,
          bootstrapUser,
        },
        { onSuccess: succeed },
      );
    } else if (source.mode === "disk") {
      add.mutate(
        { path: primaryInput, directoryName: null, bootstrapUser },
        { onSuccess: succeed },
      );
    }
  };

  return (
    <Dialog open={isOpen} onClose={close} title="Add project" className="max-w-lg">
      <form
        className="space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <RepoSourcePicker
          value={source}
          onChange={setSource}
          modes={["clone", "disk"]}
          disabled={pending}
        />

        {configMissing && (
          <Field label="Your name">
            <TextInput
              value={user}
              onChange={(e) => setUser(e.target.value)}
              placeholder="e.g. jane"
              disabled={pending}
            />
            <p className="mt-1 text-xs text-ink-dim">
              Attributes your thought documents (one-time, first-run setup).
            </p>
          </Field>
        )}

        {error != null && <ErrorNote error={error} />}

        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={close} disabled={pending}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" busy={pending} disabled={!canSubmit}>
            {pending
              ? source.mode === "clone"
                ? "Cloning…"
                : "Importing…"
              : source.mode === "clone"
                ? "Clone repo"
                : "Add project"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
