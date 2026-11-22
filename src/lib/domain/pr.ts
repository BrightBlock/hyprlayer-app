import type { PrLookup, PrLookupFailure, PrStatus } from "@/lib/bindings";

export function knownPr(lookup: PrLookup | undefined): PrStatus | null {
  if (lookup?.kind === "found") return lookup.pr;
  if (lookup?.kind === "failed") return lookup.last;
  return null;
}

export function prFailureText(cause: PrLookupFailure): string {
  switch (cause.kind) {
    case "ghMissing":
      return "gh is not installed";
    case "signedOut":
      return "gh is not signed in";
    case "rateLimited":
      if (cause.resumesAt === null) return "GitHub quota low";
      return `GitHub quota low, refreshing at ${new Date(cause.resumesAt).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      })}`;
    case "other":
      return "gh failed";
  }
}
