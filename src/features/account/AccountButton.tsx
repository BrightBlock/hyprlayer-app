import { useEffect, useRef, useState } from "react";

import { HyprlayerMark } from "@/components/Logo";
import { cx } from "@/components/ui";
import { initials } from "@/lib/format";
import {
  useAccountCancelSignIn,
  useAccountSignIn,
  useAccountSignOut,
  useAccountStatus,
  useConnectionStatus,
} from "@/lib/queries";

import { AccountPopover } from "./AccountPopover";

/** The header's account button; owns the sign-in mutations so a pending sign-in outlives the popover. */
export function AccountButton() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const status = useAccountStatus();
  const signIn = useAccountSignIn();
  const cancel = useAccountCancelSignIn();
  const signOut = useAccountSignOut();
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const connected = useConnectionStatus().data?.state === "connected";
  const shown = open && connected;

  useEffect(() => {
    if (!connected && open) setOpen(false);
  }, [connected, open]);

  const data = status.data;
  const signedInState = data?.state === "signedIn";
  const name = (data?.state === "signedIn" ? data.name.trim() : "") || null;
  const email = (data?.state === "signedIn" ? data.email?.trim() : "") || null;
  const letters = name ? initials(name) : email ? (Array.from(email)[0] ?? "").toUpperCase() || null : null;
  const who = name ?? email;
  const fallback = signedInState || status.isError ? "Account" : "Sign in";

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label={letters ? `Account: ${who}` : fallback}
        title={letters ? (who ?? undefined) : fallback === "Sign in" ? "Sign in with GitHub" : fallback}
        aria-haspopup="dialog"
        aria-expanded={shown}
        className={cx(
          "inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-control",
          "hover:bg-edge/60 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
          shown && "bg-accent-soft hover:bg-accent-soft",
        )}
      >
        {letters ? (
          <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-accent text-ui-sm font-semibold leading-none text-accent-ink">
            {letters}
          </span>
        ) : (
          <HyprlayerMark size={20} tone="muted" />
        )}
      </button>
      <AccountPopover
        open={shown}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        status={status}
        signIn={signIn}
        cancel={cancel}
        signOut={signOut}
      />
    </>
  );
}
