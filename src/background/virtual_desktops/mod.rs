pub mod bridge;
pub mod cli;
pub mod events;
pub mod handlers;
pub mod wallpapers;
mod workspace_prompt;

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use seelen_core::state::{DesktopWorkspace, VirtualDesktops, WorkspaceId};
use seelen_core::system_state::MonitorId;
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{
    SW_FORCEMINIMIZE, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOZORDER,
};

use crate::error::{Result, ResultLogExt};
use crate::event_manager;
use crate::hook::HookManager;
use crate::modules::apps::application::{UserAppWinEvent, UserAppsManager};
use crate::modules::monitors::{MonitorManager, MonitorManagerEvent};
use crate::utils::lock_free::{SyncHashMap, SyncVec, TracedMutex};
use crate::virtual_desktops::wallpapers::WorkspaceWallpapersManager;
use crate::windows_api::MonitorEnumerator;
use crate::windows_api::window::Window;
use crate::windows_api::window::event::WinEvent;

use events::VirtualDesktopEvent;

static WORKSPACES_MANAGER: LazyLock<VdManager> = LazyLock::new(VdManager::create);

/// Windows minimized by workspace switching, excluding user-minimized windows.
pub static MINIMIZED_BY_WORKSPACES: LazyLock<scc::HashSet<isize>> =
    LazyLock::new(scc::HashSet::new);
pub static RESTORED_EVENT_QUEUE: LazyLock<SyncVec<isize>> = LazyLock::new(SyncVec::new);

/// Lock order: state -> derived indexes -> UserApps. Never take state while holding an index.
pub struct VdManager {
    pub state: TracedMutex<VirtualDesktops>,
    pub workspace_index: SyncHashMap<WorkspaceId, MonitorId>,
    pub window_index: SyncHashMap<isize, WorkspaceId>,
    switching: AtomicU32,
}
event_manager!(VdManager, VirtualDesktopEvent);

impl VdManager {
    pub fn instance() -> &'static Self {
        &WORKSPACES_MANAGER
    }

    fn create() -> Self {
        let mut manager = Self::from(bridge::load_stored().unwrap_or_default());
        let previously_connected = manager.state.lock().connected_monitors.clone();
        manager.initialize().log_error();
        wallpapers::WorkspaceWallpapersManager::init(&manager);
        let added = manager
            .state
            .lock()
            .connected_monitors
            .iter()
            .filter(|id| !previously_connected.contains(id))
            .cloned()
            .collect();
        manager.prompt_for_monitors(added);
        manager
    }

    fn initialize(&mut self) -> Result<()> {
        self.reconcile_monitors();
        {
            let mut state = self.state.lock();
            state.sanitize();
            for workspace in state.workspaces.values_mut() {
                workspace
                    .windows
                    .retain(|window| Window::from(*window).is_interactable_and_not_hidden());
            }
            state
                .pinned
                .retain(|window| Window::from(*window).is_interactable_and_not_hidden());
            self.replace_indexes(&state);
            let active = state.active_workspace_ids();
            for (id, workspace) in &state.workspaces {
                if !active.contains(id) {
                    for window in &workspace.windows {
                        let _ = MINIMIZED_BY_WORKSPACES.insert_sync(*window);
                    }
                    workspace.hide(true);
                }
            }
            self.restore_visible(&state);
        }
        let windows = UserAppsManager::instance()
            .interactable_windows
            .map(|data| data.hwnd);
        for hwnd in windows {
            let window = Window::from(hwnd);
            if !window.is_minimized() {
                self.add_to_current_workspace(&window);
            }
        }
        MonitorManager::subscribe(|event| match event {
            MonitorManagerEvent::ViewAdded(_) | MonitorManagerEvent::ViewRemoved(_) => {
                let manager = Self::instance();
                let added = manager.reconcile_monitors();
                workspace_prompt::retain_connected();
                manager.restore_visible(&manager.state.lock());
                WorkspaceWallpapersManager::update_workspace_wallpapers_internal(manager);
                Self::send(VirtualDesktopEvent::StateChanged);
                manager.prompt_for_monitors(added);
            }
            _ => {}
        });
        UserAppsManager::subscribe(|event| match event {
            UserAppWinEvent::Added(addr) => {
                Self::instance().add_to_current_workspace(&Window::from(addr))
            }
            UserAppWinEvent::Removed(addr) => Self::instance().remove(&Window::from(addr)),
            _ => {}
        });
        let id =
            HookManager::subscribe(|(event, origin)| Self::on_win_event(event, origin).log_error());
        HookManager::set_event_handler_priority(&id, 2);
        Self::subscribe(|_| bridge::request_save());
        bridge::request_save();
        Ok(())
    }

    fn reconcile_monitors(&self) -> Vec<MonitorId> {
        let connected = MonitorManager::instance().get_cached_ids();
        let names: Vec<_> = MonitorEnumerator::enumerate_win32()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|display| display.get_stable_info().ok())
            .collect();
        let mut state = self.state.lock();
        for (id, name) in names {
            state.monitor_names.insert(id, name);
        }
        let added = state.reconcile_monitors(connected);
        self.replace_indexes(&state);
        added
    }

    fn prompt_for_monitors(&self, added: Vec<MonitorId>) {
        if !crate::state::application::FULL_STATE
            .load()
            .settings
            .share_workspaces_between_monitors
        {
            return;
        }
        for target in added {
            workspace_prompt::show(self, target).log_error();
        }
    }

    fn restore_workspace_in(&self, state: &VirtualDesktops, id: &WorkspaceId) {
        let owners: Vec<_> = state
            .monitors_for_workspace(id)
            .into_iter()
            .filter(|id| state.connected_monitors.contains(id))
            .collect();
        if let (Some(target), Some(workspace)) = (owners.first(), state.workspaces.get(id)) {
            workspace.restore_on_monitor(target, &owners);
        }
    }

    fn restore_visible(&self, state: &VirtualDesktops) {
        let mut active: Vec<_> = state.active_workspace_ids().into_iter().collect();
        active.sort_by(|a, b| a.0.cmp(&b.0));
        for id in active {
            self.restore_workspace_in(state, &id);
        }
    }

    fn reuse_monitor_grid(
        &self,
        target: &MonitorId,
        source: &MonitorId,
        expected: &WorkspaceId,
    ) -> Result<()> {
        self.configure_monitor_grid(target, Some(source), Some(expected))
    }

    pub fn set_monitor_workspace_grid(
        &self,
        target: &MonitorId,
        source: Option<&MonitorId>,
    ) -> Result<()> {
        self.configure_monitor_grid(target, source, None)?;
        workspace_prompt::dismiss(target);
        Ok(())
    }

    fn configure_monitor_grid(
        &self,
        target: &MonitorId,
        source: Option<&MonitorId>,
        expected: Option<&WorkspaceId>,
    ) -> Result<()> {
        let (moved, workspace, connected) = {
            let mut state = self.state.lock();
            state.connected_monitors = MonitorManager::instance().get_cached_ids();
            if let Some(expected) = expected {
                if source
                    .and_then(|source| state.monitors.get(source))
                    .is_none_or(|monitor| monitor.active_workspace_id() != expected)
                {
                    return Err("Workspace grid changed while choosing".into());
                }
            }
            let before = state.active_workspace_ids();
            let moved = if expected.is_some() {
                state.reuse_monitor_grid(target, source.ok_or("Source monitor not found")?)?
            } else {
                state.set_monitor_workspace_grid(target, source)?
            };
            let workspace = state.monitors[target].active_workspace_id().clone();
            self.replace_indexes(&state);
            let after = state.active_workspace_ids();
            for id in before.difference(&after) {
                if let Some(workspace) = state.workspaces.get(id) {
                    workspace.hide(false);
                }
            }
            self.restore_visible(&state);
            self.restore_workspace_in(&state, &workspace);
            let connected = state.connected_monitors.contains(target);
            (moved, workspace, connected)
        };
        for window in moved {
            Self::send(VirtualDesktopEvent::WindowMoved {
                window,
                desktop: workspace.clone(),
            });
        }
        WorkspaceWallpapersManager::update_workspace_wallpapers_internal(self);
        Self::send(VirtualDesktopEvent::DesktopChanged {
            monitor: target.clone(),
            workspace: workspace.clone(),
        });
        if connected {
            self.switch_to_id(&workspace)?;
        }
        Ok(())
    }

    pub fn get_workspace_of_window(&self, window: isize) -> Option<WorkspaceId> {
        self.window_index.get(&window, |id| id.clone())
    }
    pub fn get_monitor_of_workspace(&self, workspace: &WorkspaceId) -> Option<MonitorId> {
        self.workspace_index.get(workspace, |id| id.clone())
    }

    pub fn active_workspace_for_monitor(&self, monitor: &MonitorId) -> Option<WorkspaceId> {
        self.state
            .lock()
            .monitors
            .get(monitor)
            .map(|monitor| monitor.active_workspace_id().clone())
    }
    pub fn is_pinned(&self, window: &isize) -> bool {
        self.state.lock().pinned.contains(window)
    }
    fn replace_indexes(&self, state: &VirtualDesktops) {
        let (workspaces, windows) = bridge::get_indexes(state);
        self.workspace_index.replace(workspaces);
        self.window_index.replace(windows);
    }

    fn add_to_current_workspace(&self, window: &Window) {
        let addr = window.address();
        let monitor = window.monitor().stable_id();
        let added = {
            let mut state = self.state.lock();
            if state.pinned.contains(&addr)
                || self.window_index.contains_key(&addr)
                || !UserAppsManager::instance().contains_win(window)
            {
                return;
            }
            let Ok(monitor) = monitor else {
                state.pinned.push(addr);
                return;
            };
            state.ensure_monitor(monitor.clone());
            let id = state.monitors[&monitor].active_workspace_id().clone();
            state.workspaces.get_mut(&id).unwrap().windows.push(addr);
            self.replace_indexes(&state);
            id
        };
        Self::send(VirtualDesktopEvent::WindowAdded {
            window: addr,
            desktop: added,
        });
    }

    fn remove(&self, window: &Window) {
        let addr = window.address();
        let removed = {
            let mut state = self.state.lock();
            let mut removed = self.window_index.contains_key(&addr);
            state.pinned.retain(|id| {
                removed |= *id == addr;
                *id != addr
            });
            for workspace in state.workspaces.values_mut() {
                workspace.windows.retain(|id| *id != addr);
            }
            self.replace_indexes(&state);
            removed
        };
        if removed {
            Self::send(VirtualDesktopEvent::WindowRemoved { window: addr });
        }
    }

    pub fn switch_to_id(&self, workspace: &WorkspaceId) -> Result<()> {
        let connected = MonitorManager::instance().get_cached_ids();
        let together = crate::state::application::FULL_STATE
            .load()
            .settings
            .switch_workspaces_together;
        let switched = {
            let mut state = self.state.lock();
            let snapshot = state.clone();
            let before = state.active_workspace_ids();
            let changed = state.switch_to_workspace(workspace, &connected, together)?;
            if changed.is_empty() {
                return Ok(());
            }
            self.replace_indexes(&state);
            self.switching.fetch_add(1, Ordering::SeqCst);
            let mut switching = snapshot.clone();
            switching.switching = true;
            Self::send(VirtualDesktopEvent::SwitchingDesktop(switching));
            let after = state.active_workspace_ids();
            for id in before.difference(&after) {
                if let Some(workspace) = state.workspaces.get(id) {
                    workspace.hide(false);
                }
            }
            for id in state
                .workspaces
                .keys()
                .filter(|id| !snapshot.workspaces.contains_key(id))
            {
                Self::send(VirtualDesktopEvent::DesktopCreated(id.clone()));
            }
            for id in after.difference(&before) {
                if id != workspace {
                    self.restore_workspace_in(&state, id);
                }
            }
            self.restore_workspace_in(&state, workspace);
            for (monitor, workspace) in changed {
                Self::send(VirtualDesktopEvent::DesktopChanged { monitor, workspace });
            }
            true
        };
        if switched {
            std::thread::sleep(std::time::Duration::from_millis(300));
            if self.switching.fetch_sub(1, Ordering::SeqCst) == 1 {
                Self::send(VirtualDesktopEvent::SwitchingFinished);
            }
        }
        Ok(())
    }

    pub fn send_to(&self, window: &Window, workspace: &WorkspaceId) -> Result<()> {
        let addr = window.address();
        let moved = {
            let mut state = self.state.lock();
            let Some(current) = self.get_workspace_of_window(addr) else {
                return Ok(());
            };
            if &current == workspace {
                return Ok(());
            }
            if !state.workspaces.contains_key(workspace)
                || state.monitors_for_workspace(workspace).is_empty()
            {
                return Err("Workspace not found".into());
            }
            for definition in state.workspaces.values_mut() {
                definition.windows.retain(|id| *id != addr);
            }
            state
                .workspaces
                .get_mut(workspace)
                .unwrap()
                .windows
                .push(addr);
            self.replace_indexes(&state);
            if !state.active_workspace_ids().contains(workspace) {
                let _ = MINIMIZED_BY_WORKSPACES.insert_sync(addr);
                window.show_window(SW_MINIMIZE).log_error();
            }
            true
        };
        if moved {
            Self::send(VirtualDesktopEvent::WindowMoved {
                window: addr,
                desktop: workspace.clone(),
            });
        }
        Ok(())
    }

    pub fn create_desktop(&self, monitor: &MonitorId, as_row: bool) -> Result<WorkspaceId> {
        let (id, created) = {
            let mut state = self.state.lock();
            let before: std::collections::HashSet<_> = state.workspaces.keys().cloned().collect();
            let id = state.create_workspace(monitor, as_row)?;
            self.replace_indexes(&state);
            let created: Vec<_> = state
                .workspaces
                .keys()
                .filter(|id| !before.contains(id))
                .cloned()
                .collect();
            (id, created)
        };
        WorkspaceWallpapersManager::update_workspace_wallpapers_internal(self);
        for id in created {
            Self::send(VirtualDesktopEvent::DesktopCreated(id));
        }
        Ok(id)
    }

    pub fn destroy_desktop(&self, id: &WorkspaceId) -> Result<()> {
        let (removed, moved, changed) = {
            let mut state = self.state.lock();
            let before = state.clone();
            let removed = state.remove_workspace(id)?;
            self.replace_indexes(&state);
            let mut moved = Vec::new();
            for removed_id in &removed {
                if let Some(old) = before.workspaces.get(removed_id) {
                    for window in &old.windows {
                        if let Some(target) = self.get_workspace_of_window(*window) {
                            moved.push((*window, target));
                        }
                    }
                }
            }
            let changed: Vec<_> = state
                .monitors
                .iter()
                .filter_map(|(monitor_id, monitor)| {
                    let old = before.monitors.get(monitor_id)?;
                    (old.active_workspace_id() != monitor.active_workspace_id())
                        .then(|| (monitor_id.clone(), monitor.active_workspace_id().clone()))
                })
                .collect();
            let active = state.active_workspace_ids();
            let touched: std::collections::HashSet<_> = moved
                .iter()
                .map(|(_, id)| id.clone())
                .chain(changed.iter().map(|(_, id)| id.clone()))
                .collect();
            for id in touched.intersection(&active) {
                self.restore_workspace_in(&state, id);
            }
            (removed, moved, changed)
        };
        for (window, desktop) in moved {
            Self::send(VirtualDesktopEvent::WindowMoved { window, desktop });
        }
        for id in removed {
            Self::send(VirtualDesktopEvent::DesktopDestroyed(id));
        }
        for (monitor, workspace) in changed {
            Self::send(VirtualDesktopEvent::DesktopChanged { monitor, workspace });
        }
        Self::send(VirtualDesktopEvent::StateChanged);
        Ok(())
    }

    pub fn rename_desktop(&self, id: &WorkspaceId, name: Option<String>) -> Result<()> {
        self.state
            .lock()
            .workspaces
            .get_mut(id)
            .ok_or("Workspace not found")?
            .name = name;
        Self::send(VirtualDesktopEvent::StateChanged);
        Ok(())
    }
}

pub trait DesktopWorkspaceExt {
    fn hide(&self, force: bool);
    fn restore(&self);
    fn restore_on_monitor(&self, monitor_id: &MonitorId, owners: &[MonitorId]);
}

impl DesktopWorkspaceExt for DesktopWorkspace {
    fn restore_on_monitor(&self, monitor_id: &MonitorId, owners: &[MonitorId]) {
        self.restore();
        // Windows can leave undocked windows on a remaining laptop display. Move
        // restored windows to their replacement without unminimizing user-minimized ones.
        let Ok(monitors) = MonitorEnumerator::enumerate_win32() else {
            return;
        };
        let Some(target) = monitors
            .into_iter()
            .find(|monitor| monitor.stable_id().ok().as_ref() == Some(monitor_id))
            .and_then(|monitor| monitor.rect().ok())
        else {
            return;
        };
        for addr in &self.windows {
            let window = Window::from(*addr);
            if window.is_minimized()
                || window
                    .monitor()
                    .stable_id()
                    .ok()
                    .is_some_and(|id| owners.contains(&id))
            {
                continue;
            }
            if let Ok(rect) = window.outer_rect() {
                let width = rect.width().clamp(1, target.width().max(1));
                let height = rect.height().clamp(1, target.height().max(1));
                let destination = RECT {
                    left: target.left,
                    top: target.top,
                    right: target.left + width,
                    bottom: target.top + height,
                };
                window
                    .set_position(&destination, SWP_NOACTIVATE | SWP_NOZORDER)
                    .log_error();
            }
        }
    }

    fn hide(&self, force: bool) {
        let mode = if force { SW_FORCEMINIMIZE } else { SW_MINIMIZE };
        for addr in &self.windows {
            let window = Window::from(*addr);
            if window.is_window() && !window.is_minimized() {
                let _ = MINIMIZED_BY_WORKSPACES.insert_sync(window.address());
                window.show_window(mode).log_error();
            }
        }
    }

    fn restore(&self) {
        let len = self.windows.len();
        for (idx, addr) in self.windows.iter().enumerate() {
            let window = Window::from(*addr);
            let is_minimized = window.is_minimized();

            // avoid restore windows manually minimized by the user
            if is_minimized && !MINIMIZED_BY_WORKSPACES.contains_sync(addr) {
                continue;
            }

            if is_minimized {
                // Push before show_window to avoid a race where SystemMinimizeEnd
                // fires on the hook thread before this thread reaches the push.
                RESTORED_EVENT_QUEUE.push(*addr);
                // use normal show instead async cuz it will keep the order of restoring
                window.show_window(SW_RESTORE).log_error();
            }
            MINIMIZED_BY_WORKSPACES.remove_sync(addr);

            // ensure correct focus
            if idx == len - 1 {
                window.focus().log_error();
            }
        }
    }
}

impl VdManager {
    fn move_to_top(&self, window: isize) {
        let mut updated = false;
        {
            let mut state = self.state.lock();
            for workspace in state.workspaces.values_mut() {
                if workspace.windows.contains(&window) {
                    workspace.windows.retain(|id| *id != window);
                    workspace.windows.push(window);
                    updated = true;
                    break;
                }
            }
        }
        if updated {
            bridge::request_save();
        }
    }

    fn on_user_unminimize(window: &Window) -> Result<()> {
        let manager = Self::instance();
        let addr = window.address();
        let Some(workspace) = manager.get_workspace_of_window(addr) else {
            manager.add_to_current_workspace(window);
            return Ok(());
        };
        let monitor = manager
            .get_monitor_of_workspace(&workspace)
            .ok_or("Workspace has no monitor")?;
        if MonitorManager::instance()
            .get_cached_ids()
            .contains(&monitor)
        {
            manager.move_to_top(addr);
            manager.switch_to_id(&workspace)?;
            let state = manager.state.lock();
            manager.restore_workspace_in(&state, &workspace);
        } else if crate::state::application::FULL_STATE
            .load()
            .settings
            .share_workspaces_between_monitors
        {
            MINIMIZED_BY_WORKSPACES.remove_sync(&addr);
        } else {
            MINIMIZED_BY_WORKSPACES.remove_sync(&addr);
            let monitor = window.monitor().stable_id()?;
            let target = manager
                .state
                .lock()
                .monitors
                .get(&monitor)
                .map(|m| m.active_workspace_id().clone());
            if let Some(target) = target {
                manager.send_to(window, &target)?;
            }
        }
        Ok(())
    }

    fn on_win_event(event: WinEvent, window: Window) -> Result<()> {
        let addr = window.address();
        match event {
            WinEvent::SystemMinimizeEnd => {
                let mut restored = false;
                RESTORED_EVENT_QUEUE.retain(|id| {
                    if !restored && *id == addr {
                        restored = true;
                        false
                    } else {
                        true
                    }
                });
                if restored {
                    return Ok(());
                }
                let result = Self::on_user_unminimize(&window);
                Self::send(VirtualDesktopEvent::WindowUnminimizedByUser { window: addr });
                result?;
            }
            WinEvent::ObjectDestroy => {
                RESTORED_EVENT_QUEUE.retain(|id| *id != addr);
                MINIMIZED_BY_WORKSPACES.remove_sync(&addr);
            }
            WinEvent::SystemForeground | WinEvent::ObjectFocus => {
                Self::instance().move_to_top(addr)
            }
            WinEvent::SynDebouncedRectChange => {
                let manager = Self::instance();
                if manager.is_pinned(&addr) {
                    return Ok(());
                }
                let Some(workspace) = manager.get_workspace_of_window(addr) else {
                    return Ok(());
                };
                let (owners, connected) = {
                    let state = manager.state.lock();
                    (
                        state.monitors_for_workspace(&workspace),
                        state.connected_monitors.clone(),
                    )
                };
                let Ok(physical) = window.monitor().stable_id() else {
                    return Ok(());
                };
                // A shared workspace owns one window set across all of its connected displays.
                if owners.contains(&physical) {
                    return Ok(());
                }
                if owners.iter().all(|id| !connected.contains(id))
                    && crate::state::application::FULL_STATE
                        .load()
                        .settings
                        .share_workspaces_between_monitors
                {
                    return Ok(());
                }
                let target = manager
                    .state
                    .lock()
                    .monitors
                    .get(&physical)
                    .map(|m| m.active_workspace_id().clone());
                if let Some(target) = target {
                    manager.send_to(&window, &target)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
