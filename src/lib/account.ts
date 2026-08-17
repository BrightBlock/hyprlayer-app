/**
 * Hyprlayer cloud accounts haven't shipped, so nothing is signed in yet. Every
 * account-gated surface asks here, so sign-in lands in exactly one place.
 */
export function useSignedIn(): boolean {
  return false;
}
