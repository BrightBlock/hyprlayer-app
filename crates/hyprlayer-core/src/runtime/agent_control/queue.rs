use hyprlayer_models::{LeadQueueItem, LeadQueueState, RuntimeEvent};

use super::{blocking, AgentControlService};
use crate::error::{CoreError, Result};

fn one_line(field: &str, text: &str) -> Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return Err(CoreError::LeadQueueInvalid {
            detail: format!("a queue item needs a {field}"),
        });
    }
    Ok(text)
}

impl AgentControlService<'_> {
    pub async fn lead_queue(&self, project_path: &str) -> Result<Vec<LeadQueueItem>> {
        let (db, path) = (self.runtime.db.clone(), project_path.to_string());
        blocking(move || db.lead_queue(&path)).await
    }

    pub async fn add_lead_queue_item(
        &self,
        project_path: &str,
        title: &str,
        summary: &str,
    ) -> Result<LeadQueueItem> {
        let (title, summary) = (one_line("title", title)?, one_line("summary", summary)?);
        let (db, path) = (self.runtime.db.clone(), project_path.to_string());
        let item = blocking(move || db.add_lead_queue_item(&path, &title, &summary)).await?;
        self.announce_queue(project_path);
        Ok(item)
    }

    pub async fn start_lead_queue_item(
        &self,
        project_path: &str,
        id: &str,
        workstream_id: &str,
    ) -> Result<LeadQueueItem> {
        let (db, path, item_id, ws) = (
            self.runtime.db.clone(),
            project_path.to_string(),
            id.to_string(),
            workstream_id.to_string(),
        );
        let started = blocking(move || {
            let ours = db
                .get_workstream(&ws)?
                .is_some_and(|w| w.project_path == path);
            if !ours {
                return Err(CoreError::LeadQueueInvalid {
                    detail: format!("workstream {ws} is not in this project"),
                });
            }
            match db.lead_queue_item(&path, &item_id)? {
                Some(item) if item.state == LeadQueueState::Queued => {}
                Some(item) => {
                    return Err(CoreError::LeadQueueInvalid {
                        detail: format!("item {item_id} is {}, not queued", item.state.as_str()),
                    })
                }
                None => return Err(CoreError::LeadQueueItemGone { id: item_id }),
            }
            db.set_lead_queue_state(&path, &item_id, LeadQueueState::Running, Some(&ws))
        })
        .await?
        .ok_or_else(|| CoreError::LeadQueueItemGone { id: id.into() })?;
        self.announce_queue(project_path);
        Ok(started)
    }

    pub async fn finish_lead_queue_item(
        &self,
        project_path: &str,
        id: &str,
    ) -> Result<LeadQueueItem> {
        let (db, path, item_id) = (
            self.runtime.db.clone(),
            project_path.to_string(),
            id.to_string(),
        );
        let done =
            blocking(move || db.set_lead_queue_state(&path, &item_id, LeadQueueState::Done, None))
                .await?
                .ok_or_else(|| CoreError::LeadQueueItemGone { id: id.into() })?;
        self.announce_queue(project_path);
        Ok(done)
    }

    pub async fn remove_lead_queue_item(&self, project_path: &str, id: &str) -> Result<()> {
        let (db, path, item_id) = (
            self.runtime.db.clone(),
            project_path.to_string(),
            id.to_string(),
        );
        if !blocking(move || db.remove_lead_queue_item(&path, &item_id)).await? {
            return Err(CoreError::LeadQueueItemGone { id: id.into() });
        }
        self.announce_queue(project_path);
        Ok(())
    }

    pub async fn reorder_lead_queue(&self, project_path: &str, ids: Vec<String>) -> Result<()> {
        let (db, path) = (self.runtime.db.clone(), project_path.to_string());
        blocking(move || db.reorder_lead_queue(&path, &ids)).await?;
        self.announce_queue(project_path);
        Ok(())
    }

    fn announce_queue(&self, project_path: &str) {
        let _ = self.runtime.events.send(RuntimeEvent::LeadQueueChanged {
            project_path: project_path.to_string(),
        });
    }
}
