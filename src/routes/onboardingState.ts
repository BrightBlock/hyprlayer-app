// Module-scoped dismissed flag for the onboarding wizard. When the user closes
// the wizard without completing it, this prevents the auto-launch from re-popping
// during the same session. Resets on a cold app reload (desired).
//
// Lives in its own (non-component) module so the routes that read/mutate it —
// `Root.tsx` and `Onboarding.tsx` — keep all-component exports and stay eligible
// for React Fast Refresh. ESM live bindings mean a reader sees mutations made
// through `dismissOnboarding()`.
export let onboardingDismissed = false;

export function dismissOnboarding() {
  onboardingDismissed = true;
}
