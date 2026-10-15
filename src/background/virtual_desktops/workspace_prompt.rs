use std::{
    collections::HashMap,
    sync::{LazyLock, Once},
};

use parking_lot::Mutex;
use seelen_core::{
    state::{CssStyles, Dialog, DialogContent, VirtualDesktops, WorkspaceId},
    system_state::MonitorId,
};
use tauri::{Listener, Manager};
use uuid::Uuid;

use crate::{
    app::get_app_handle,
    error::{Result, ResultLogExt},
    modules::monitors::MonitorManager,
    state::application::FULL_STATE,
    widgets::{trigger_dialog_backend, webview::WidgetWebviewLabel},
};

use super::VdManager;

static PROMPTS: LazyLock<Mutex<HashMap<MonitorId, WorkspacePrompt>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SETTINGS_LISTENER: Once = Once::new();

struct WorkspacePrompt {
    id: Uuid,
    listeners: Vec<tauri::EventId>,
}

impl WorkspacePrompt {
    fn close(&self) {
        let label = WidgetWebviewLabel::new(&"@seelen/dialog".into(), None, Some(&self.id));
        if let Some(window) = get_app_handle().get_webview_window(&label.raw) {
            window.close().log_error();
        }
    }
}

impl Drop for WorkspacePrompt {
    fn drop(&mut self) {
        for listener in self.listeners.drain(..) {
            get_app_handle().unlisten(listener);
        }
    }
}

pub(super) fn show(manager: &VdManager, target: MonitorId) -> Result<()> {
    SETTINGS_LISTENER.call_once(|| {
        get_app_handle().listen(
            seelen_core::handlers::SeelenEvent::StateSettingsChanged,
            |_| {
                retain_connected();
            },
        );
    });
    let connected = MonitorManager::instance().get_cached_ids();
    if !connected.contains(&target) || !FULL_STATE.load().settings.share_workspaces_between_monitors
    {
        return Ok(());
    }
    let state: VirtualDesktops = manager.into();
    // Shared references persist across reconnections; an already linked monitor
    // does not need to choose the same definitions again.
    if state.monitors.get(&target).is_some_and(|target_grid| {
        state
            .monitors
            .iter()
            .any(|(id, monitor)| id != &target && monitor.workspaces == target_grid.workspaces)
    }) {
        return Ok(());
    }
    let sources = state.reusable_monitor_grids(&connected);
    if sources.is_empty() {
        return Ok(());
    }
    let mut pending = PROMPTS.lock();
    if pending.contains_key(&target) {
        return Ok(());
    }
    let mut prompt = WorkspacePrompt {
        id: Uuid::new_v4(),
        listeners: Vec::new(),
    };
    let id = prompt.id;
    let separate_event = format!("workspace-grid-{id}-separate");
    let target_name = state
        .monitor_names
        .get(&target)
        .cloned()
        .unwrap_or_else(|| t!("workspace_grid.connected_monitor").to_string());
    let mut choices = Vec::new();
    for (index, source) in sources.into_iter().enumerate() {
        let grid = &state.monitors[&source];
        let expected = grid.active_workspace_id().clone();
        let source_name = state
            .monitor_names
            .get(&source)
            .cloned()
            .unwrap_or_else(|| t!("workspace_grid.saved_monitor", number = index + 1).to_string());
        let count = grid.workspaces.rows().iter().map(Vec::len).sum::<usize>();
        let preview = grid
            .workspaces
            .rows()
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .map(move |(column, workspace)| (row, column, workspace))
            })
            .map(|(row, column, id)| {
                state
                    .workspaces
                    .get(id)
                    .and_then(|workspace| workspace.name.clone())
                    .unwrap_or_else(|| {
                        t!(
                            "workspace_grid.position",
                            row = row + 1,
                            column = column + 1
                        )
                        .to_string()
                    })
            })
            .take(3)
            .collect::<Vec<_>>()
            .join(", ");
        let event = format!("workspace-grid-{id}-reuse-{index}");
        let response_target = target.clone();
        prompt
            .listeners
            .push(get_app_handle().listen(&event, move |_| {
                resolve(
                    response_target.clone(),
                    id,
                    Some((source.clone(), expected.clone())),
                );
            }));
        choices.push(button(
            t!(
                "workspace_grid.reuse",
                monitor = source_name,
                count = count,
                preview = preview
            )
            .to_string(),
            event,
            "default",
        ));
    }
    let response_target = target.clone();
    prompt
        .listeners
        .push(get_app_handle().listen(&separate_event, move |_| {
            resolve(response_target.clone(), id, None);
        }));
    let dialog = Dialog {
        identifier: id,
        width: 520.0,
        height: 340.0,
        title: vec![text(
            t!("workspace_grid.title", monitor = target_name).to_string(),
        )],
        content: vec![
            text(t!("workspace_grid.body").to_string()),
            DialogContent::Group {
                items: choices,
                styles: Some(
                    CssStyles::new()
                        .add("display", "flex")
                        .add("flexDirection", "column")
                        .add("gap", "var(--spacing-xs)"),
                ),
            },
        ],
        footer: vec![button(
            t!("workspace_grid.separate").to_string(),
            separate_event.clone(),
            "solid",
        )],
        on_close: Some(separate_event),
    };
    pending.insert(target.clone(), prompt);
    drop(pending);
    if let Err(error) = trigger_dialog_backend(dialog) {
        PROMPTS.lock().remove(&target);
        return Err(error);
    }
    Ok(())
}

pub(super) fn retain_connected() {
    let connected = MonitorManager::instance().get_cached_ids();
    let enabled = FULL_STATE.load().settings.share_workspaces_between_monitors;
    let removed: Vec<_> = {
        let mut pending = PROMPTS.lock();
        let ids: Vec<_> = pending
            .keys()
            .filter(|id| !enabled || !connected.contains(id))
            .cloned()
            .collect();
        ids.into_iter()
            .filter_map(|id| pending.remove(&id))
            .collect()
    };
    for prompt in removed {
        prompt.close();
    }
}

fn resolve(target: MonitorId, id: Uuid, source: Option<(MonitorId, WorkspaceId)>) {
    let prompt = {
        let mut pending = PROMPTS.lock();
        if pending.get(&target).is_none_or(|prompt| prompt.id != id) {
            return;
        }
        pending.remove(&target)
    };
    // Unregister every button listener, including choices that were not selected.
    drop(prompt);
    if !FULL_STATE.load().settings.share_workspaces_between_monitors {
        return;
    }
    if let Some((source, expected)) = source {
        let manager = VdManager::instance();
        if let Err(error) = manager.reuse_monitor_grid(&target, &source, &expected) {
            log::warn!("Workspace grid selection expired: {error}");
            // Another prompt or a new connection may have consumed this source.
            // Refresh the choices rather than adopting a different grid silently.
            show(manager, target).log_error();
        }
    }
}

fn text(value: String) -> DialogContent {
    DialogContent::Text {
        value,
        styles: None,
    }
}

fn button(value: String, on_click: String, skin: &str) -> DialogContent {
    DialogContent::Button {
        skin: Some(skin.to_string()),
        inner: vec![text(value)],
        styles: None,
        on_click,
    }
}
