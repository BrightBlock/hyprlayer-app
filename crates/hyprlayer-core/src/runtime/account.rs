use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hyprlayer_models::RuntimeEvent;
use serde::{Deserialize, Serialize};

use crate::account::cognito::{self, Identity, RefreshOutcome, Refusal, Tokens};
use crate::account::endpoints::CloudEndpoints;
use crate::account::pkce::{self, Pkce};
use crate::account::store::{self, StoredAccount};
use crate::error::{CoreError, Result};

use super::ports::{ProviderHttpError, ProviderHttpRequest};
use super::HyprlayerRuntime;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_MAX_BYTES: usize = 64 * 1024;
const ATTEMPT_TTL: Duration = Duration::from_secs(10 * 60);
const FRESH_TIMEOUT: Duration = Duration::from_secs(20);
const REVOKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Seconds before expiry at which a token counts as stale.
const STALE_MARGIN: u64 = 60;
const TRANSIENT_BACKOFF: Duration = Duration::from_secs(60);
/// The longest a deleted or revoked user stays signed in.
pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum AccountStatus {
    SignedOut,
    SignedIn { name: String, email: Option<String> },
}

/// The browser leg's inputs; the PKCE verifier stays in the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignInStart {
    pub state: String,
    pub logout_url: String,
    pub authorize_url: String,
}

struct PendingSignIn {
    state: String,
    verifier: String,
    redirect_uri: String,
    created: Instant,
}

pub(crate) struct AccountState {
    endpoints: CloudEndpoints,
    /// One attempt at a time: a new start supersedes the old.
    pending: Mutex<Option<PendingSignIn>>,
    /// Serializes every `account.json` read-modify-write.
    file: tokio::sync::Mutex<()>,
    /// Answers `status()` while a refresh holds `file`.
    last: Mutex<AccountStatus>,
    last_transient: Mutex<Option<Instant>>,
}

impl AccountState {
    pub(crate) fn new(endpoints: CloudEndpoints, app_config_dir: &Path) -> Self {
        let last = store::read(&store::path(app_config_dir))
            .filter(|account| account.belongs_to(&endpoints))
            .map_or(AccountStatus::SignedOut, |account| status_of(&account));
        Self {
            endpoints,
            pending: Mutex::new(None),
            file: tokio::sync::Mutex::new(()),
            last: Mutex::new(last),
            last_transient: Mutex::new(None),
        }
    }
}

fn status_of(account: &StoredAccount) -> AccountStatus {
    AccountStatus::SignedIn {
        name: account.name.clone(),
        email: account.email.clone(),
    }
}

enum Refresh {
    Done,
    SignedOut,
    Transient,
}

fn ensure_running(runtime: &HyprlayerRuntime) -> Result<()> {
    if runtime.coordinator.is_stopped() {
        return Err(CoreError::RuntimeStopped);
    }
    Ok(())
}

pub struct AccountService<'a> {
    runtime: &'a HyprlayerRuntime,
}

impl HyprlayerRuntime {
    pub fn account(&self) -> AccountService<'_> {
        AccountService { runtime: self }
    }
}

impl AccountService<'_> {
    /// May refresh a stale token; never waits behind a refresh already in flight.
    pub async fn status(&self) -> Result<AccountStatus> {
        ensure_running(self.runtime)?;
        match self.state().file.try_lock() {
            Ok(file) => self.ensure_fresh(&file, false).await,
            Err(_) => Ok(self.cached()),
        }
    }

    /// Forces a refresh: Cognito refuses a deleted or revoked user's refresh token at
    /// once, while `/v1/session` keeps accepting the access token until it expires.
    pub(crate) async fn check(&self) {
        if ensure_running(self.runtime).is_err() || self.read_own().is_none() {
            return;
        }
        let file = self.state().file.lock().await;
        if let Err(e) = self.ensure_fresh(&file, true).await {
            tracing::warn!("account check failed: {e}");
        }
    }

    pub async fn start_sign_in(&self, port: u16) -> Result<SignInStart> {
        ensure_running(self.runtime)?;
        if port == 0 {
            return Err(CoreError::AccountSignInFailed {
                detail: "start_sign_in needs the loopback's bound port, not 0".into(),
            });
        }
        let endpoints = &self.state().endpoints;
        let pkce = Pkce::generate()?;
        let state = pkce::random_token()?;
        let start = SignInStart {
            logout_url: cognito::logout_url(endpoints, port)?,
            authorize_url: cognito::authorize_url(endpoints, &pkce.challenge, &state, port)?,
            state: state.clone(),
        };
        *self.pending() = Some(PendingSignIn {
            state,
            verifier: pkce.verifier,
            redirect_uri: cognito::redirect_uri(port),
            created: Instant::now(),
        });
        Ok(start)
    }

    pub async fn finish_sign_in(&self, state: &str, code: &str) -> Result<AccountStatus> {
        ensure_running(self.runtime)?;
        let attempt = self.take_attempt(state)?;
        let tokens = self.exchange_code(code, &attempt).await?;
        let identity = self.fetch_identity(&tokens.access_token).await?;
        let endpoints = &self.state().endpoints;
        let status = AccountStatus::SignedIn {
            name: identity.name.clone(),
            email: identity.email.clone(),
        };
        let account = StoredAccount {
            client_id: endpoints.client_id.clone(),
            login_url: endpoints.login_url.clone(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: now_secs().saturating_add(tokens.expires_in),
            name: identity.name,
            email: identity.email,
        };
        {
            let _file = self.state().file.lock().await;
            store::write(&self.path(), &account)?;
            *self.last() = status.clone();
            *self.last_transient() = None;
        }
        let _ = self.runtime.events.send(RuntimeEvent::AccountChanged);
        Ok(status)
    }

    pub async fn sign_out(&self) -> Result<AccountStatus> {
        ensure_running(self.runtime)?;
        let removed = {
            let _file = self.state().file.lock().await;
            let removed = match self.read_own() {
                Some(account) => {
                    self.revoke(&account.refresh_token).await;
                    store::delete(&self.path())?
                }
                None => false,
            };
            *self.last() = AccountStatus::SignedOut;
            *self.last_transient() = None;
            removed
        };
        if removed {
            let _ = self.runtime.events.send(RuntimeEvent::AccountChanged);
        }
        Ok(AccountStatus::SignedOut)
    }

    fn state(&self) -> &AccountState {
        &self.runtime.account_state
    }

    fn path(&self) -> PathBuf {
        store::path(&self.runtime.paths.app_config_dir)
    }

    fn pending(&self) -> MutexGuard<'_, Option<PendingSignIn>> {
        self.state()
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn last(&self) -> MutexGuard<'_, AccountStatus> {
        self.state()
            .last
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn last_transient(&self) -> MutexGuard<'_, Option<Instant>> {
        self.state()
            .last_transient
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn cached(&self) -> AccountStatus {
        self.last().clone()
    }

    /// Records `status` as current, announcing it only when it differs from the last one.
    fn settle(&self, status: AccountStatus) -> AccountStatus {
        let mut last = self.last();
        if *last != status {
            *last = status.clone();
            let _ = self.runtime.events.send(RuntimeEvent::AccountChanged);
        }
        status
    }

    fn read_own(&self) -> Option<StoredAccount> {
        store::read(&self.path()).filter(|account| account.belongs_to(&self.state().endpoints))
    }

    /// Any attached client may call finish (ADR-0009), so a stray or forged state must
    /// leave the attempt in flight alone; only an exact, unexpired match takes it.
    fn take_attempt(&self, state: &str) -> Result<PendingSignIn> {
        let mut pending = self.pending();
        pending.take_if(|attempt| attempt.created.elapsed() >= ATTEMPT_TTL);
        pending
            .take_if(|attempt| attempt.state == state)
            .ok_or(CoreError::AccountSignInStale)
    }

    async fn exchange_code(&self, code: &str, attempt: &PendingSignIn) -> Result<Tokens> {
        let endpoints = &self.state().endpoints;
        let form =
            cognito::code_exchange_form(endpoints, code, &attempt.verifier, &attempt.redirect_uri);
        let response = self
            .runtime
            .provider_http
            .post_form(cloud_request(cognito::token_url(endpoints)), form)
            .await
            .map_err(|e| unreachable_error("token endpoint", e))?;
        cognito::classify_token_response(response.status, &response.body)
            .map_err(|refusal| refused("token endpoint", refusal))
    }

    async fn fetch_identity(&self, access_token: &str) -> Result<Identity> {
        self.request_session(access_token)
            .await
            .map_err(|e| unreachable_error("session", e))?
            .map_err(|refusal| refused("session", refusal))
    }

    async fn request_session(
        &self,
        access_token: &str,
    ) -> std::result::Result<std::result::Result<Identity, Refusal>, ProviderHttpError> {
        let request = cloud_request(cognito::session_url(&self.state().endpoints))
            .header("authorization", format!("Bearer {access_token}"));
        let response = self.runtime.provider_http.get(request).await?;
        Ok(cognito::parse_session(response.status, &response.body))
    }

    /// `_file` proves the caller holds `account.json`'s lock for the whole run.
    async fn ensure_fresh(
        &self,
        _file: &tokio::sync::MutexGuard<'_, ()>,
        force: bool,
    ) -> Result<AccountStatus> {
        match tokio::time::timeout(FRESH_TIMEOUT, self.refresh_if_stale(force)).await {
            Ok(status) => status,
            Err(_) => {
                tracing::warn!("account refresh timed out; keeping the cached session");
                Ok(self.cached())
            }
        }
    }

    async fn refresh_if_stale(&self, force: bool) -> Result<AccountStatus> {
        let Some(mut account) = self.read_own() else {
            return Ok(self.settle(AccountStatus::SignedOut));
        };
        if force || now_secs().saturating_add(STALE_MARGIN) >= account.expires_at {
            if self.backing_off() {
                return Ok(self.cached());
            }
            match self.refresh(&mut account).await? {
                Refresh::Done => self.update_identity(&mut account).await?,
                Refresh::SignedOut => return Ok(self.settle(AccountStatus::SignedOut)),
                Refresh::Transient => return Ok(self.cached()),
            }
        }
        Ok(self.settle(status_of(&account)))
    }

    fn backing_off(&self) -> bool {
        self.last_transient()
            .is_some_and(|failed| failed.elapsed() < TRANSIENT_BACKOFF)
    }

    async fn refresh(&self, account: &mut StoredAccount) -> Result<Refresh> {
        let endpoints = &self.state().endpoints;
        let form = cognito::refresh_form(endpoints, &account.refresh_token);
        let outcome = match self
            .runtime
            .provider_http
            .post_form(cloud_request(cognito::token_url(endpoints)), form)
            .await
        {
            Ok(response) => cognito::classify_refresh_response(response.status, &response.body),
            Err(_) => RefreshOutcome::Transient("unreachable".into()),
        };
        Ok(match outcome {
            RefreshOutcome::Refreshed(tokens) => {
                account.access_token = tokens.access_token;
                account.expires_at = now_secs().saturating_add(tokens.expires_in);
                store::write(&self.path(), account)?;
                *self.last_transient() = None;
                tracing::info!("account session refreshed");
                Refresh::Done
            }
            RefreshOutcome::InvalidGrant => {
                store::delete(&self.path())?;
                tracing::info!("account refresh answered invalid_grant; signed out");
                Refresh::SignedOut
            }
            RefreshOutcome::Transient(detail) => {
                *self.last_transient() = Some(Instant::now());
                tracing::info!("account refresh failed ({detail}); keeping the session");
                Refresh::Transient
            }
        })
    }

    /// A rejected or failed check keeps the cached identity.
    async fn update_identity(&self, account: &mut StoredAccount) -> Result<()> {
        match self.request_session(&account.access_token).await {
            Ok(Ok(identity)) => {
                if identity.name != account.name || identity.email != account.email {
                    account.name = identity.name;
                    account.email = identity.email;
                    store::write(&self.path(), account)?;
                }
            }
            Ok(Err(refusal)) => tracing::warn!(
                "account session check answered {}; keeping the cached identity",
                refusal.status
            ),
            Err(_) => tracing::warn!("account session check failed; keeping the cached identity"),
        }
        Ok(())
    }

    async fn revoke(&self, refresh_token: &str) {
        let endpoints = &self.state().endpoints;
        let request = cloud_request(cognito::revoke_url(endpoints)).timeout(REVOKE_TIMEOUT);
        let form = cognito::revoke_form(endpoints, refresh_token);
        match self.runtime.provider_http.post_form(request, form).await {
            Ok(response) if (200..300).contains(&response.status) => {
                tracing::info!("account refresh token revoked");
            }
            Ok(response) => tracing::warn!("account revoke answered {}", response.status),
            Err(_) => tracing::warn!("account revoke could not reach the Cloud"),
        }
    }
}

fn cloud_request(url: String) -> ProviderHttpRequest {
    ProviderHttpRequest::new(url)
        .timeout(HTTP_TIMEOUT)
        .max_bytes(HTTP_MAX_BYTES)
}

fn unreachable_error(what: &str, e: ProviderHttpError) -> CoreError {
    CoreError::AccountUnreachable {
        detail: format!("{what}: {e}"),
    }
}

/// A 5xx is the service's failure; any other refusal is this sign-in's.
fn refused(what: &str, refusal: Refusal) -> CoreError {
    let detail = match refusal.error {
        Some(error) => format!("{what} answered {} ({error})", refusal.status),
        None => format!("{what} answered {}", refusal.status),
    };
    if refusal.status >= 500 {
        CoreError::AccountUnreachable { detail }
    } else {
        CoreError::AccountSignInFailed { detail }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}
