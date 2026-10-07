import { useAccountStatus } from "./queries";

export function useSignedIn(): boolean {
  return useAccountStatus().data?.state === "signedIn";
}
