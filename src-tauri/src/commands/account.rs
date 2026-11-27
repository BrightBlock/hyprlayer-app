use hyprlayer_client::account::{self, SignInFlight};
use hyprlayer_core::runtime::account::AccountStatus;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Notify;

use crate::error::{AppError, CmdResult};
use crate::state::AppState;

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
    client: std::sync::Arc<crate::client::RpcClient>,
    cancel: &Notify,
) -> CmdResult<AccountStatus> {
    // Vite binds [::1] on its port, so the loopback must not take it.
    let dev_port = if cfg!(debug_assertions) {
        app.config().build.dev_url.as_ref().and_then(|u| u.port())
    } else {
        None
    };
    account::browser_leg(
        client,
        dev_port,
        |url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|e| e.to_string())
        },
        || state.client(),
        cancel,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub fn account_cancel_sign_in(flight: State<'_, SignInFlight>) {
    flight.cancel();
}
