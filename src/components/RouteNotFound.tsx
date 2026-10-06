import { useNavigate, useRouterState } from "@tanstack/react-router";

import { BLANK_CTA, BlankState } from "./BlankState";
import { Button } from "./ui";

// A restored last route from another build (dev identifiers are shared across branches) lands here.
export function RouteNotFound() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const navigate = useNavigate();
  return (
    <BlankState
      art="session"
      align="center"
      title={
        <>
          Nothing here.
        </>
      }
      lede={<span className="font-mono text-ui">{pathname}</span>}
      actions={
        <Button variant="primary" className={BLANK_CTA} onClick={() => void navigate({ to: "/" })}>
          Go to dashboard
        </Button>
      }
    />
  );
}
