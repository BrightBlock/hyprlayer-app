import { useId, type RefObject } from "react";
import type { UseMutationResult, UseQueryResult } from "@tanstack/react-query";

import { BrandMark } from "@/components/BrandMark";
import { BAR_POPOVER, BAR_POPOVER_ROW, ErrorNote, MenuSeparator, Popover, ROW_ACTION, Spinner } from "@/components/ui";
import type { AccountStatus } from "@/lib/bindings";
import { isAppError } from "@/lib/ipc";

const ROW = "flex items-center gap-2.5 px-3 py-2 text-ui text-ink-2";

export function AccountPopover({
  open,
  onClose,
  anchorRef,
  status,
  signIn,
  cancel,
  signOut,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  status: UseQueryResult<AccountStatus>;
  signIn: UseMutationResult<AccountStatus, unknown, void>;
  cancel: UseMutationResult<void, unknown, void>;
  signOut: UseMutationResult<AccountStatus, unknown, void>;
}) {
  const headingId = useId();
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      align="end"
      sideOffset={10}
      caret
      trapFocus
      aria-labelledby={headingId}
      className={BAR_POPOVER}
    >
      <div className="px-3 pb-1.5 pt-1">
        <span id={headingId} className="lbl">
          Account
        </span>
      </div>
      <Body status={status} signIn={signIn} cancel={cancel} signOut={signOut} />
    </Popover>
  );
}

function Body({
  status,
  signIn,
  cancel,
  signOut,
}: {
  status: UseQueryResult<AccountStatus>;
  signIn: UseMutationResult<AccountStatus, unknown, void>;
  cancel: UseMutationResult<void, unknown, void>;
  signOut: UseMutationResult<AccountStatus, unknown, void>;
}) {
  if (status.isError) return <ErrorNote error={status.error} />;
  if (!status.data) {
    return (
      <div className={ROW}>
        <Spinner size="sm" />
      </div>
    );
  }
  if (status.data.state === "signedIn") {
    const { name, email } = status.data;
    return (
      <>
        <div className="px-3 py-1.5">
          <div className="text-ui text-ink">{name}</div>
          {email && <div className="text-meta text-ink-dim">{email}</div>}
        </div>
        <MenuSeparator />
        <button
          type="button"
          className={`${BAR_POPOVER_ROW} w-full`}
          disabled={signOut.isPending}
          onClick={() => signOut.mutate()}
        >
          Sign out
        </button>
        {signOut.error != null && <ErrorNote error={signOut.error} onDismiss={signOut.reset} />}
      </>
    );
  }
  // A cancelled attempt is the user's own doing, not a failure.
  const failure = signIn.error != null && !isCancelled(signIn.error) ? signIn.error : null;
  return (
    <>
      {signIn.isPending ? (
        <div className={`${ROW} justify-between`}>
          <span className="flex items-center gap-2.5">
            <Spinner size="sm" />
            Waiting for GitHub…
          </span>
          <button type="button" className={ROW_ACTION} onClick={() => cancel.mutate()}>
            Cancel
          </button>
        </div>
      ) : (
        <button type="button" className={`${BAR_POPOVER_ROW} w-full`} onClick={() => signIn.mutate()}>
          <BrandMark brand="github" size="sm" />
          Sign in with GitHub
        </button>
      )}
      {failure != null && <ErrorNote error={failure} onDismiss={signIn.reset} />}
    </>
  );
}

const isCancelled = (e: unknown) => isAppError(e) && e.code === "account_sign_in_cancelled";
