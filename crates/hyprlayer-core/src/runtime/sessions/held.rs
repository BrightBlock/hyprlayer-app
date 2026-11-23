use hyprlayer_models::agent_control::AgentCaller;
use hyprlayer_models::session::ClientRef;

use crate::agent::AgentDriver;
use crate::runtime::HyprlayerRuntime;
use crate::store::HeldMessage;

use super::service::blocking;

const EXCERPT_CHARS: usize = 200;

/// Who sent an agent's message, so it hears back if the message is never read.
#[derive(Debug, Clone)]
pub struct MessageSender {
    pub caller: AgentCaller,
    pub project_path: String,
}

/// A message no client keeps a copy of, recorded until claude takes it in.
pub(super) struct Hold<'a> {
    pub nonce: &'a str,
    pub text: &'a str,
    pub prompt_name: Option<&'a str>,
    pub author: Option<ClientRef>,
    pub sender: Option<MessageSender>,
}

pub(super) async fn hold(rt: &HyprlayerRuntime, session_id: &str, hold: Hold<'_>) {
    let (sender, project_path) = match hold.sender {
        Some(sender) => (Some(sender.caller), Some(sender.project_path)),
        None => (None, None),
    };
    let held = HeldMessage {
        session_id: session_id.to_string(),
        nonce: hold.nonce.to_string(),
        text: hold.text.to_string(),
        prompt_name: hold.prompt_name.map(str::to_string),
        author: hold.author,
        sender,
        project_path,
        resent: false,
    };
    let db = rt.db.clone();
    if let Err(e) = blocking(move || db.hold_message(&held)).await {
        tracing::warn!("session {session_id}: a held message was not recorded: {e}");
    }
}

pub(super) async fn release(rt: &HyprlayerRuntime, session_id: &str, nonce: &str) {
    let (db, sid, nonce) = (rt.db.clone(), session_id.to_string(), nonce.to_string());
    if let Err(e) = blocking(move || db.release_held(&sid, &nonce)).await {
        tracing::warn!("session {session_id}: a delivered message stays recorded as held: {e}");
    }
}

/// Tells each sender its message was never read, and forgets the messages. With `told_only`,
/// a message no one would be told about stays for the next relaunch to resend.
pub(crate) async fn report_undelivered(
    rt: &HyprlayerRuntime,
    session_id: &str,
    told_only: bool,
    why: &str,
) {
    let (db, sid) = (rt.db.clone(), session_id.to_string());
    match blocking(move || db.take_held(&sid, told_only)).await {
        Ok(held) => report(rt, held, why),
        Err(e) => tracing::warn!("session {session_id}: its held messages were not read: {e}"),
    }
}

/// A message recorded after its driver left: the relay's report, or a relaunch's resend, may
/// already have run without it.
pub(super) async fn recover_late_hold(rt: &HyprlayerRuntime, session_id: &str, nonce: &str) {
    let (db, sid) = (rt.db.clone(), session_id.to_string());
    let held = match blocking(move || db.held_messages(&sid)).await {
        Ok(held) => held
            .into_iter()
            .find(|held| held.nonce == nonce && !held.resent),
        Err(e) => {
            tracing::warn!("session {session_id}: its held messages were not read: {e}");
            return;
        }
    };
    let Some(held) = held else { return };
    match rt.session_registry.get(session_id).await {
        Some(driver) => {
            send_again(
                rt,
                session_id,
                driver.as_ref(),
                held,
                "the relaunched session refused it",
            )
            .await
        }
        None if held.sender.is_some() => {
            release(rt, session_id, nonce).await;
            report(rt, vec![held], "the session exited before reading it");
        }
        None => {}
    }
}

/// On a relaunch: sends each held message again, once, in order; one already resent is reported lost.
pub(super) async fn resend(rt: &HyprlayerRuntime, session_id: &str, driver: &dyn AgentDriver) {
    let (db, sid) = (rt.db.clone(), session_id.to_string());
    let (resend, lost) = match blocking(move || db.relaunch_held(&sid)).await {
        Ok(held) => held,
        Err(e) => {
            tracing::warn!("session {session_id}: its held messages were not read: {e}");
            return;
        }
    };
    report(
        rt,
        lost,
        "it was sent again after the session restarted, and the session stopped again before reading it",
    );
    for held in resend {
        send_again(
            rt,
            session_id,
            driver,
            held,
            "the relaunched session refused it",
        )
        .await;
    }
}

pub(super) async fn send_dropped(rt: &HyprlayerRuntime, session_id: &str, nonces: Vec<String>) {
    let Some(driver) = rt.session_registry.get(session_id).await else {
        return;
    };
    let (db, sid) = (rt.db.clone(), session_id.to_string());
    let held = match blocking(move || db.held_messages(&sid)).await {
        Ok(held) => held,
        Err(e) => {
            tracing::warn!("session {session_id}: its held messages were not read: {e}");
            return;
        }
    };
    for held in held.into_iter().filter(|h| nonces.contains(&h.nonce)) {
        send_again(
            rt,
            session_id,
            driver.as_ref(),
            held,
            "the session dropped it unread and then refused it",
        )
        .await;
    }
}

async fn send_again(
    rt: &HyprlayerRuntime,
    session_id: &str,
    driver: &dyn AgentDriver,
    held: HeldMessage,
    refused: &str,
) {
    let reserved = rt
        .session_actors
        .reserve_user_turn(
            session_id,
            held.nonce.clone(),
            None,
            held.author.clone(),
            false,
        )
        .await;
    let sent = reserved.and_then(|()| {
        driver.send_user_turn(
            &held.text,
            None,
            held.prompt_name.as_deref(),
            Some(&held.nonce),
        )
    });
    match sent {
        Ok(()) => tracing::info!("session {session_id}: sent a held message again"),
        Err(e) => {
            tracing::warn!("session {session_id}: a held message was not sent again: {e}");
            // One no one would hear about waits for the next relaunch instead.
            if held.sender.is_some() {
                release(rt, session_id, &held.nonce).await;
            }
            report(rt, vec![held], refused);
        }
    }
}

/// After a restart's wakes: messages held by a session nothing relaunched will never be read.
pub(crate) async fn sweep_after_restart(rt: &HyprlayerRuntime) {
    let db = rt.db.clone();
    let ids = match blocking(move || db.held_session_ids()).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("held messages were not checked after the restart: {e}");
            return;
        }
    };
    for id in ids {
        if !rt.sessions().is_live(&id).await {
            report_undelivered(
                rt,
                &id,
                false,
                "hyprlayer restarted and did not resume that session",
            )
            .await;
        }
    }
}

fn report(rt: &HyprlayerRuntime, held: Vec<HeldMessage>, why: &str) {
    for message in held {
        let (Some(sender), Some(project_path)) = (message.sender, message.project_path) else {
            tracing::warn!(
                "session {}: a message no one sent was not delivered: {why}",
                message.session_id
            );
            continue;
        };
        let excerpt: String = message.text.chars().take(EXCERPT_CHARS).collect();
        let more = if message.text.chars().count() > EXCERPT_CHARS {
            "…"
        } else {
            ""
        };
        tracing::info!(
            "session {}: a message from {} was not delivered: {why}",
            message.session_id,
            sender.id()
        );
        rt.agent_control().report_to(
            sender,
            project_path,
            format!(
                "Your message to session {} was not delivered: {why}. It read:\n\n{excerpt}{more}",
                message.session_id
            ),
        );
    }
}
