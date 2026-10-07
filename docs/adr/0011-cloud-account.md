# ADR 0011: The server holds the Hyprlayer Cloud session; the desktop runs the browser leg

**Status:** proposed · 2026-10-06

## Decision

Signing in to Hyprlayer Cloud is Cognito's authorization-code flow with PKCE, federated to
GitHub by the backend (`identity_provider=GitHubOidc`). Each `hyprlayer-server` owns its
profile's session. The desktop only carries the browser through the redirects.

| | Owns |
|---|---|
| Server (`AccountService`, `runtime/account.rs`) | the PKCE verifier and `state`, the code exchange, the tokens, `/v1/session`, `account.json`, `AccountChanged` |
| Desktop (core's `account::loopback`, run by the shell) | a loopback port, the system browser, the redirect back from Cognito |

**The loopback is a core primitive the shell runs.** The browser resolves `localhost` on its
own machine, the desktop's, so the redirect listener must be there; a remote server could
never hear it. `Loopback` binds a port free on both `127.0.0.1` and `::1` (a port taken on
`::1` alone is busy: Vite listens there), answers `/auth/loggedout` with a redirect to the
authorize URL, and hands back only a callback carrying the attempt's `state`. The server still
listens on nothing (ADR-0009). The callback's request is held open
until `account_finish_sign_in` answers: only then is it redirected to
`https://hyprlayer.dev/signin/success`, a static page with no code or state, and a failed
exchange gets a local failure page instead. The browser never shows a Cognito or OIDC page, and
a cancelled sign-in gets a local message. Four RPC commands carry the rest: `account_start_sign_in(port)`
returns `{state, logout_url, authorize_url}`, `account_finish_sign_in(state, code)` exchanges
the code, and `account_status` and `account_sign_out` read and end the session.

**The verifier never leaves the server.** `start_sign_in` keeps one attempt; a new start
supersedes it. `finish_sign_in` takes it only on an exact `state` under ten minutes old, once.
Any attached client may call finish, so a mismatch is `account_sign_in_stale` and leaves the
attempt in place; an expired attempt is cleared. The file is written and the event sent only
after both the exchange and `/v1/session` succeed.

**`/v1/session` is the identity.** Its `author.name` (the GitHub display name, else the login)
and `author.email` are what the server stores and returns. The ID token is never stored or
parsed.

**`account.json`** sits in `<app_config_dir>` on each server host: `clientId`, `loginUrl`,
`accessToken`, `refreshToken`, `expiresAt`, `name`, `email`. It is written through a temp file
created 0600 on unix, then renamed; Windows gets no mode bits, like OpenCode's `auth.json`.
`clientId` and `loginUrl` name the pool, because a debug and a release server can share an
`app_config_dir`: a file from the other pool reads as signed out, and status never rewrites it
nor sign-out deletes it.

**The pool follows the build profile.** Debug builds use the dev pool. Every release-profile
build, releases and dev installs alike, uses prod. There is no switch: releases are built
locally, and a switch left set in a release shell would ship a server signed into the dev pool.
Each pool is reached through its custom hosted-UI domain, the only one the GitHub federation
admits.

| Pool | Hosted UI | Client id | API |
|---|---|---|---|
| dev | `https://login.dev.cloud.hyprlayer.dev` | `5qu3dne9cln4ugit8i2aeupv1h` | `https://api.dev.cloud.hyprlayer.dev` |
| prod | `https://login.cloud.hyprlayer.dev` | `6chvucvj17rrg8dosl2dvmjvkk` | `https://api.cloud.hyprlayer.dev` |

**Network.** `ProviderHttp` form-POSTs as well as GETs, still to `String`. `account::cognito`
builds every request and reads every answer without I/O, and `runtime/account.rs` sends them,
because a module outside `runtime/` may not reach the port (ADR-0008). A transport failure or a
5xx is `account_unreachable`; any other refusal is `account_sign_in_failed`. Their `detail`,
logged by the server and never sent to a client, holds the status and the OAuth `error` code
only. No form, header or body is logged, and the `Authorization` header is marked sensitive.

**`AccountChanged` is capability-targeted.** It carries nothing; a client re-reads
`account_status`. It reaches every connection that announced the kind, whatever it watches, and
no other. That is what makes a new event kind safe here despite ADR-0010's reason for adding
none: a desktop older than 0.8.6 tears its connection down on an unknown kind, and the 0.8.6 and
0.8.7 desktops announce only the three session kinds, so no released desktop receives it.

**`MIN_SERVER_VERSION` is 0.8.9.** *(Amended 2026-10-07 — it was left at 0.8.8 so an
optional feature would not replace a running server.)* A server older than 0.8.9 would keep a
deleted or revoked user signed in with no periodic check, so the desktop requires the server
that runs it. The cost is paid once: updating to this desktop replaces a local server older than
0.8.9, ending its live sessions and terminals.

## Session lifecycle

`status()` and a periodic check keep the session fresh; both run `ensure_fresh` holding the
`account.json` lock for the whole run, under one 20 s timeout, so concurrent callers share one
refresh. `status()` never waits behind it: when a refresh holds the lock it answers from the
last status kept in memory, which is filled from the file at open, so a restarted server never
flashes signed out. At runtime start and every hour after, a task forces a refresh of a
stored session. A refresh is the check, not `/v1/session`: on 2026-10-07, against the dev pool, a
session whose refresh grant already answered `invalid_grant` still got 200 from `/v1/session`
for its unexpired 15-minute access token. A deleted or revoked user is therefore signed out
within an hour, sooner if `status()` refreshes an expired token first, at one refresh and one
`/v1/session` per hour per signed-in server.

| Situation | Behavior |
|---|---|
| No file, or a foreign file | signed out |
| Token stale (`now + 60 s >= expires_at`), or the periodic check | refresh, unless a transient refresh failure happened in the last 60 s; then the cached status |
| Refresh answers `invalid_grant` (a 400) | delete the file, send `AccountChanged`, signed out |
| Refresh fails any other way (network, 5xx, other 4xx) | keep the file, back off 60 s, cached status |
| Refreshed | `GET /v1/session` with the new token to pick up identity changes; a rejection or a failure keeps the cached identity and warns |
| Any outcome | changed tokens or identity are written atomically; `AccountChanged` is sent only when the status differs from the cached one |

Only `invalid_grant` signs the user out. Cognito returns no refresh token on a refresh, so the
stored one is kept. Sign-out revokes the refresh token with a 5 s timeout before deleting the
file; a failed revoke is logged and the user is signed out regardless.

## Known behavior

- Re-sign-in about monthly: the refresh token lasts 30 days, with no sliding window.
- A GitHub name change appears on the next sign-in.
- Debug and release builds sign in to different pools.

## Test knobs

`HYPRLAYER_CLOUD_LOGIN_URL`, `HYPRLAYER_CLOUD_API_URL` and `HYPRLAYER_CLOUD_CLIENT_ID` replace
the pool's endpoints and client id. Unlike ADR-0010's skills knobs they redirect bearer tokens,
so their reader is compiled into debug builds only: a release binary carries neither the code
nor the names. They are read only through `RuntimeEnvironment::var`, never the env overlay, and
each warns once when set. The URLs accept `https://` or loopback `http://`, the skills API
knob's rule, with a trailing `/` trimmed; the client id must be non-empty. Anything else is
ignored. The `account_sign_in` E2E scenario points them at an in-process stub.

## What we're not doing

- No keychain: the tokens live in `account.json`.
- No `hyprlayer://` deep link; the loopback is the only redirect.
- No email, Google or SSO sign-in: GitHub only.
- No pool switch beyond the debug-only test knobs.
- No sign-in shared across server hosts: each has its own `account.json`.
