/**
 * One live-activity signal per workstream, folded from every session bound to
 * it. Drives the sidebar activity dot, so you can see which workstreams are
 * running, blocked on you, or waiting for review without opening each one.
 *
 * A workstream takes the most urgent state any of its sessions is in. Highest
 * priority first:
 *  - `needs-you`:  parked on a permission grant or a question, and stuck until
 *                  you answer. The state most worth surfacing.
 *  - `working`:    computing a turn.
 *  - `error`:      crashed or failed to spawn.
 *  - `review`:     turn finished or session completed. Nothing is running, but
 *                  a result is waiting.
 *  - `background`: the turn is done, but background shells are still running.
 *                  Gets its own pulse, separate from `working` and `review`.
 *  - `idle`:       nothing worth flagging, including cold start before sessions
 *                  are restored into the store.
 *
 * See `useWorkstreamActivity` (features/session/sessionStore.ts) for the
 * per-session fold and `ActivityDot` (components/ActivityDot.tsx) for the
 * shared presentation.
 *
 * A hand-declared TS union, not a Rust binding — nothing on the backend
 * models "activity", it is derived client-side from session state — but two
 * features (session, search) render it, so ADR-0004's rule applies: it
 * graduates to `lib/domain` rather than staying feature-owned.
 */
export type WorkstreamActivity = "needs-you" | "working" | "error" | "background" | "review" | "idle";
