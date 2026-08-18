import { useState } from "react";

import { githubOwner } from "@/lib/github";

function initial(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

/**
 * A repo's owner avatar, the GitHub org/user icon, served from
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
