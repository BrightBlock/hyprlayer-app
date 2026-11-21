import { useState } from "react";

import { EmptyState, IconButton, TextInput } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { openExternalUrl } from "@/lib/browser";
import { log } from "@/lib/log";

const PLAIN_HTTP_HOST = /^(localhost(?=[:/]|$)|127\.|\d{1,3}(\.\d{1,3}){3}(?=[:/]|$)|\[[\da-f:]+\]|[^/:]+:\d+(?=\/|$))/i;

function normalizePreviewUrl(raw: string): string | null {
  const value = raw.trim();
  if (!value) return null;
  const withScheme = /^[a-z][a-z\d+.-]*:\/\//i.test(value)
    ? value
    : `${PLAIN_HTTP_HOST.test(value) ? "http" : "https"}://${value}`;
  try {
    const url = new URL(withScheme);
    // A same-origin frame escapes allow-scripts + allow-same-origin and reaches the app's IPC.
    if (url.origin === window.location.origin) return null;
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

export function WebPreviewTab({
  url,
  onNavigate,
}: {
  url: string;
  onNavigate: (url: string) => void;
}) {
  const [input, setInput] = useState(url);
  const [reloads, setReloads] = useState(0);

  function commit() {
    const next = normalizePreviewUrl(input);
    if (!next) return;
    setInput(next);
    if (next === url) setReloads((n) => n + 1);
    else onNavigate(next);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-edge px-2">
        <TextInput
          aria-label="URL"
          placeholder="localhost:3000"
          autoFocus={!url}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
          }}
          className="min-w-0 flex-1 py-1 text-ui"
        />
        <IconButton
          aria-label="Reload"
          title="Reload"
          disabled={!url}
          onClick={() => setReloads((n) => n + 1)}
        >
          <Icon name="retry" />
        </IconButton>
        <IconButton
          aria-label="Open in browser"
          title="Open in browser"
          disabled={!url}
          onClick={() =>
            void openExternalUrl(url).catch((e) => log.warn("web-preview", "opening in a browser failed", e))
          }
        >
          <Icon name="external" />
        </IconButton>
      </div>
      {url ? (
        <iframe
          key={reloads}
          src={url}
          title="Web preview"
          sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"
          className="min-h-0 w-full flex-1 border-0 bg-white"
        />
      ) : (
        <EmptyState title="Preview a web page">
          Enter a URL, such as a local dev server on localhost:3000.
        </EmptyState>
      )}
    </div>
  );
}
