/**
 * Pure derivation of the artifacts an agent published to claude.ai from a
 * session's transcript items. Claude Code's `Artifact` tool publishes a local
 * HTML/Markdown file to a private claude.ai page and reports the URL back in
 * its tool result; that arrives as an ordinary `tool` transcript item, so no
 * backend change is needed to surface it.
 *
 * Wire shape (Claude Code 2.1.233 stream-json), captured from a real session:
 *   tool_use  name: "Artifact"
 *     input:  {"file_path":"…/out/linux-and-codex-rollout.html","description":"…",
 *              "favicon":"🐧📦","capabilities":{"artifact":{},"downloads":{}}}
 *   tool_result:
 *     "Published …/linux-and-codex-rollout.html at https://claude.ai/code/artifact/05f7c471-…
 *
 *      Live subscription: skipped — …"
 *
 * The URL only exists in the result's first line, in the form
 * `Published <file_path> at <url>`. `action` also takes `"list"`, `"comments"`,
 * `"reply"` and `"resolve"`, none of which publish anything, so only an omitted
 * `action` (or `"publish"`) counts. There is no title on the wire: the page's
 * real name lives in the source file's `<title>` tag, which the transcript
 * never sees, so the best available name is the tool's optional `title` input
 * (rare in practice) or the source filename.
 *
 * Codex (`codex-cli 0.147.0`) has no artifact tool — its nearest concept,
 * `image_generation_call`, writes local PNGs rather than hosted pages, and our
 * own item mapper doesn't forward it to the transcript at all
 * (`crates/hyprlayer-core/src/codex/protocol.rs:640-700`). OpenCode has
 * nothing either. Both harnesses simply yield an empty artifact list here.
 */

import type { TranscriptItem } from "./transcript";
import { baseName, parseToolInput } from "./toolDisplay";

export type SessionArtifact = {
  /** The published page's claude.ai URL — the identity of an artifact, since a
   *  republish keeps the URL while the file path and title can both change. */
  url: string;
  /** Display name: the tool's `title` if it passed one, else the source
   *  filename. The page's real name lives in the file's `<title>` tag, which the
   *  transcript never carries. */
  title: string;
  description: string | null;
  filePath: string;
};

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

// Anchored to the result's first line; takes the last whitespace-run token so
// a `file_path` that happens to contain " at " can't mislead the match.
const PUBLISHED = /^Published\b.*\s(https:\/\/\S+)$/;

function publishedUrl(content: string): string | null {
  return PUBLISHED.exec(content.split("\n", 1)[0].trim())?.[1] ?? null;
}

/** Strip the extension (including the design canvas's `.dc.html`) and
 *  prettify a source path into a display title, mirroring `docTitle`. */
function fileStem(path: string): string {
  const base = baseName(path);
  const stem = base.replace(/(\.dc)?\.(html?|md|markdown)$/i, "");
  return stem.replace(/[-_]+/g, " ").trim() || base;
}

/** If a tool item is a successful `Artifact` publish, the artifact it minted;
 *  else null (a non-publish action, an in-flight or errored call, or any
 *  other tool). */
export function artifactFromTool(
  name: string,
  input: string,
  result: { content: string; isError: boolean } | null,
): SessionArtifact | null {
  if (name !== "Artifact") return null;
  const obj = parseToolInput(input);
  const action = str(obj?.action);
  if (action && action !== "publish") return null;
  if (!result || result.isError) return null;

  const url = publishedUrl(result.content);
  if (!url) return null;

  const filePath = str(obj?.file_path);
  const title = str(obj?.title) || fileStem(filePath);
  const description = str(obj?.description) || null;
  return { url, title, description, filePath };
}

/**
 * Artifacts published across a session's transcript, in first-published order,
 * deduped by URL: a republish (same file, or an explicit `url` input) overwrites
 * the entry in place via `Map.set`'s insertion-order semantics, so the list
 * doesn't reshuffle mid-session while the latest title and description win.
 */
export function artifactsFromItems(items: TranscriptItem[]): SessionArtifact[] {
  const hit = artifactsCache.get(items);
  if (hit) return hit;
  const byUrl = new Map<string, SessionArtifact>();
  for (const item of items) {
    if (item.kind !== "tool") continue;
    const artifact = artifactFromTool(item.name, item.input, item.result);
    if (artifact) byUrl.set(artifact.url, artifact);
  }
  const out = [...byUrl.values()];
  artifactsCache.set(items, out);
  return out;
}

// Same memo as `sessionDocs`, and for the same reason: the workstream-wide
// selector above this walks every session on every store read, a keystroke
// included. Keyed on the `items` array `reduceEvent` replaces per event, so a
// stale entry is unreachable. No `written`-style validity key here — this
// projection depends on nothing but the items.
const artifactsCache = new WeakMap<TranscriptItem[], SessionArtifact[]>();
