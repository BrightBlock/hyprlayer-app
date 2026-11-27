//! The Cloud sign-in's browser leg (ADR-0011): bind the loopback, open the browser, wait.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyprlayer_core::account::loopback::{Callback, Loopback};
use hyprlayer_core::runtime::account::AccountStatus;
use hyprlayer_core::CoreError;
use tokio::sync::Notify;

use crate::client::RpcClient;
use crate::error::{AppError, CmdResult};

/// Registered Cognito callback ports; the backend allows only these.
pub const LOOPBACK_PORTS: [u16; 2] = [1430, 1420];
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// The one in-flight browser leg; a new attempt supersedes the old.
#[derive(Default)]
pub struct SignInFlight {
    current: Mutex<Option<Arc<Notify>>>,
}

impl SignInFlight {
    pub fn begin(&self) -> Arc<Notify> {
        let mine = Arc::new(Notify::new());
        let previous = self.current.lock().unwrap().replace(mine.clone());
        if let Some(previous) = previous {
            previous.notify_one();
        }
        mine
    }

    pub fn cancel(&self) {
        if let Some(current) = self.current.lock().unwrap().take() {
            current.notify_one();
        }
    }

    pub fn end(&self, mine: &Arc<Notify>) {
        let mut current = self.current.lock().unwrap();
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, mine)) {
            *current = None;
        }
    }
}

/// `skip_port` is a port another listener owns, such as the dev server's.
/// `open_url` opens the browser; `current_client` is read again at the end because a
/// reconnect during the browser leg replaces the client.
pub async fn browser_leg(
    client: Arc<RpcClient>,
    skip_port: Option<u16>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
    current_client: impl FnOnce() -> CmdResult<Arc<RpcClient>>,
    cancel: &Notify,
) -> CmdResult<AccountStatus> {
    let candidates: Vec<u16> = LOOPBACK_PORTS
        .into_iter()
        .filter(|p| Some(*p) != skip_port)
        .collect();
    let loopback = Loopback::bind_first(&candidates)
        .await
        .map_err(AppError::from)?;
    log::info!("account sign-in listening on port {}", loopback.port());

    let start = client
        .account_start_sign_in(loopback.port())
        .await
        .map_err(AppError::from)?;
    open_url(&start.logout_url)
        .map_err(|e| AppError::internal(format!("Couldn't open the browser: {e}")))?;

    let mut callback = loopback
        .wait(SIGN_IN_TIMEOUT, &start.state, &start.authorize_url, cancel)
        .await
        .map_err(AppError::from)?;
    let outcome = finish(current_client, start.state, &callback).await;
    callback.reply(outcome.is_ok()).await;
    outcome
}

async fn finish(
    current_client: impl FnOnce() -> CmdResult<Arc<RpcClient>>,
    attempt: String,
    callback: &Callback,
) -> CmdResult<AccountStatus> {
    let code = match (&callback.code, &callback.error) {
        (Some(code), None) => code.clone(),
        (_, error) => {
            return Err(CoreError::AccountSignInFailed {
                detail: error
                    .clone()
                    .unwrap_or_else(|| "no authorization code".into()),
            }
            .into())
        }
    };
    current_client()?
        .account_finish_sign_in(attempt, code)
        .await
        .map_err(AppError::from)
}
