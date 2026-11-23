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
            Ok(()) => tracing::info!("session {session_id}: resent a message it held at exit"),
            Err(e) => {
                tracing::warn!("session {session_id}: a held message was not resent: {e}");
                report(rt, vec![held], "the relaunched session refused it");
            }
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
