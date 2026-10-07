use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyprlayer_core::account::loopback::{Callback, Loopback};
use hyprlayer_core::runtime::account::AccountStatus;
use hyprlayer_core::CoreError;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

/// Registered Cognito callback ports; the backend allows only these.
const LOOPBACK_PORTS: [u16; 2] = [1430, 1420];
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// The one in-flight browser leg; a new attempt supersedes the old.
#[derive(Default)]
pub struct SignInFlight {
    current: Mutex<Option<Arc<Notify>>>,
}

impl SignInFlight {
    fn begin(&self) -> Arc<Notify> {
        let mine = Arc::new(Notify::new());
        let previous = self.current.lock().unwrap().replace(mine.clone());
        if let Some(previous) = previous {
            previous.notify_one();
        }
        mine
    }

    fn cancel(&self) {
        if let Some(current) = self.current.lock().unwrap().take() {
            current.notify_one();
        }
    }

    fn end(&self, mine: &Arc<Notify>) {
        let mut current = self.current.lock().unwrap();
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, mine)) {
            *current = None;
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn account_status(state: State<'_, AppState>) -> CmdResult<AccountStatus> {
    state
        .client()?
        .account_status()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn account_sign_out(state: State<'_, AppState>) -> CmdResult<AccountStatus> {
    state
        .client()?
        .account_sign_out()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub async fn account_sign_in(
    app: AppHandle,
    state: State<'_, AppState>,
    flight: State<'_, SignInFlight>,
) -> CmdResult<AccountStatus> {
    let client = state.client()?;
    let cancel = flight.begin();
    let outcome = browser_leg(&app, &state, client, &cancel).await;
    flight.end(&cancel);
    match &outcome {
        Ok(_) => log::info!("account sign-in finished"),
        Err(e) => log::info!("account sign-in ended: {}", e.code),
    }
    outcome
}

async fn browser_leg(
    app: &AppHandle,
    state: &AppState,
    client: Arc<crate::client::RpcClient>,
    cancel: &Notify,
) -> CmdResult<AccountStatus> {
    // Vite binds [::1] on its port, so the loopback must not take it.
    let dev_port = if cfg!(debug_assertions) {
        app.config().build.dev_url.as_ref().and_then(|u| u.port())
    } else {
        None
    };
    let candidates: Vec<u16> = LOOPBACK_PORTS
        .into_iter()
        .filter(|p| Some(*p) != dev_port)
        .collect();
    let loopback = Loopback::bind_first(&candidates)
        .await
        .map_err(AppError::from)?;
    log::info!("account sign-in listening on port {}", loopback.port());

    let start = client
        .account_start_sign_in(loopback.port())
        .await
        .map_err(AppError::from)?;
    app.opener()
        .open_url(&start.logout_url, None::<&str>)
        .map_err(|e| AppError::internal(format!("Couldn't open the browser: {e}")))?;

    let mut callback = loopback
        .wait(SIGN_IN_TIMEOUT, &start.state, &start.authorize_url, cancel)
        .await
        .map_err(AppError::from)?;
    let outcome = finish(state, start.state, &callback).await;
    callback.reply(outcome.is_ok()).await;
    outcome
}

async fn finish(
    state: &AppState,
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
    // A reconnect during the browser leg replaces the client.
    state
        .client()?
        .account_finish_sign_in(attempt, code)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
#[specta::specta]
pub fn account_cancel_sign_in(flight: State<'_, SignInFlight>) {
    flight.cancel();
}
