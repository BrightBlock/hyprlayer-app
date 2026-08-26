
import type { TranscriptItem } from "./transcript";
import { baseName, parseToolInput } from "./toolDisplay";

export type SessionArtifact = {
  url: string;
  title: string;
  description: string | null;
  filePath: string;
};

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

const PUBLISHED = /^Published\b.*\s(https:\/\/\S+)$/;

function publishedUrl(content: string): string | null {
  return PUBLISHED.exec(content.split("\n", 1)[0].trim())?.[1] ?? null;
}

function fileStem(path: string): string {
  const base = baseName(path);
  const stem = base.replace(/(\.dc)?\.(html?|md|markdown)$/i, "");
  return stem.replace(/[-_]+/g, " ").trim() || base;
}

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

const artifactsCache = new WeakMap<TranscriptItem[], SessionArtifact[]>();
