import { useState } from "react";

import { githubOwner } from "@/lib/github";

function initial(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

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
