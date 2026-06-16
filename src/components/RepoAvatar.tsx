import { useState } from "react";

/**
 * Extract the GitHub owner (user or org) from a clone source — a URL
 * (`https://github.com/acme/repo.git`, `git@github.com:acme/repo.git`,
 * `ssh://git@github.com/acme/repo`) or a bare `owner/repo`. Returns null for
 * non-GitHub or unparseable sources, so the caller falls back to a letter tile.
 */
export function githubOwner(source: string | null | undefined): string | null {
  if (!source) return null;
  const s = source.trim();
  // URL or scp-style: capture the segment after the github.com host.
  const host = s.match(/github\.com[/:]([A-Za-z0-9._-]+)\/[^/\s]+/i);
  if (host) return host[1];
  // Bare `owner/repo` (a nameWithOwner) — treat as GitHub.
  const bare = s.match(/^([A-Za-z0-9._-]+)\/[A-Za-z0-9._-]+$/);
  if (bare) return bare[1];
  return null;
}

function initial(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

/**
 * A repo's owner avatar — the GitHub org/user icon, served from
 * `github.com/<owner>.png` (no API call, no auth). Falls back to a letter tile
 * when the source isn't a GitHub repo (a local import) or the image can't load
 * (offline, etc.).
 */
export function RepoAvatar({
  source,
  name,
  size = 18,
  className = "rounded",
}: {
  source?: string | null;
  name: string;
  size?: number;
  className?: string;
}) {
  const owner = githubOwner(source);
  const [failed, setFailed] = useState(false);
  const dims = { width: size, height: size };

  if (owner && !failed) {
    return (
      <img
        // 2× for crisp rendering on retina displays.
        src={`https://github.com/${owner}.png?size=${size * 2}`}
        alt=""
        loading="lazy"
        onError={() => setFailed(true)}
        className={`shrink-0 object-cover ${className}`}
        style={dims}
      />
    );
  }
  return (
    <div
      aria-hidden="true"
      className={`flex shrink-0 items-center justify-center bg-edge font-semibold text-ink-dim ${className}`}
      style={{ ...dims, fontSize: Math.round(size * 0.5) }}
    >
      {initial(name)}
    </div>
  );
}
