import { useState } from "react";

export function useCopyFeedback() {
  const [copied, setCopied] = useState(false);
  const trigger = (text: string) => {
    if (!text) return;
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };
  return { copied, trigger };
}
