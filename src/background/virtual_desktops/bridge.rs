use std::{
    collections::HashMap,
    io::Write,
    sync::{
        LazyLock,
        atomic::{AtomicU32, Ordering},
    },
};

use crate::{
    error::{Result, ResultLogExt},
    utils::{
        constants::SEELEN_COMMON,
        lock_free::{SyncHashMap, TracedMutex},
    },
    virtual_desktops::VdManager,
};
use seelen_core::{
    state::{VirtualDesktops, WorkspaceId},
    system_state::MonitorId,
};
use slu_utils::{Debounce, debounce};

pub fn get_indexes(
    state: &VirtualDesktops,
) -> (HashMap<WorkspaceId, MonitorId>, HashMap<isize, WorkspaceId>) {
    let mut workspace_index = HashMap::new();
    let mut window_index = HashMap::new();

    for (id, workspace) in &state.workspaces {
        if let Some(monitor) = state.monitors_for_workspace(id).into_iter().next() {
            workspace_index.insert(id.clone(), monitor);
        }
        for window in &workspace.windows {
            window_index.insert(*window, id.clone());
        }
    }

    (workspace_index, window_index)
}

impl From<VirtualDesktops> for VdManager {
    fn from(value: VirtualDesktops) -> Self {
        let (workspace_index, window_index) = get_indexes(&value);
        Self {
            state: TracedMutex::new(value),
            switching: AtomicU32::new(0),
            workspace_index: SyncHashMap::from(workspace_index),
            window_index: SyncHashMap::from(window_index),
        }
    }
}

impl From<&VdManager> for VirtualDesktops {
    fn from(value: &VdManager) -> Self {
        let mut state = value.state.lock().clone();
        state.switching = value.switching.load(Ordering::SeqCst) > 0;
        state
    }
}

pub fn load_stored() -> Result<VirtualDesktops> {
    let path = SEELEN_COMMON.app_cache_dir().join("workspaces2.json");
    let file = std::fs::File::open(path)?;
    file.lock()?;
    Ok(serde_json::from_reader(file)?)
}

fn save_state() -> Result<()> {
    let path = SEELEN_COMMON.app_cache_dir().join("workspaces2.json");
    let state: VirtualDesktops = VdManager::instance().into();

    let mut file = std::fs::File::create(path)?;
    serde_json::to_writer(&mut file, &state)?;
    file.flush()?;

    log::trace!("desktop workspaces successfully saved");
    Ok(())
}

pub fn request_save() {
    static SAVE_DEBOUNCER: LazyLock<Debounce<()>> = LazyLock::new(|| {
        debounce(
            |_| {
                save_state().log_error();
            },
            std::time::Duration::from_secs(2),
        )
    });
    SAVE_DEBOUNCER.call(());
}
