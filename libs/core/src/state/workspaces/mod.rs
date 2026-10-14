use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::{error::Result, identifier_impl, resource::WallpaperId, system_state::MonitorId};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), ts(export))]
pub struct VirtualDesktops {
    /// Workspaces per monitor
    pub monitors: HashMap<MonitorId, VirtualDesktopMonitor>,
    /// pinned windows will be not affected by switching workspaces
    pub pinned: Vec<isize>,
    /// true if the system is currently switching workspaces
    pub switching: bool,
    /// Most recently connected displays, used to avoid duplicate connection prompts.
    pub connected_monitors: Vec<MonitorId>,
    /// Friendly display names, including disconnected displays offered for reuse.
    pub monitor_names: HashMap<MonitorId, String>,
}

impl VirtualDesktops {
    /// Switch by grid position across connected displays when synchronization is enabled.
    /// Grow smaller grids so every display can reach the requested position; each
    /// display retains its own workspace IDs and windows.
    pub fn switch_to_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
        connected: &[MonitorId],
        together: bool,
    ) -> Result<Vec<(MonitorId, WorkspaceId)>> {
        let (source, (row, column)) = self
            .monitors
            .iter()
            .find_map(|(id, monitor)| {
                monitor
                    .workspaces
                    .position(workspace_id)
                    .map(|position| (id.clone(), position))
            })
            .ok_or("Workspace not found")?;
        let mut targets = if together {
            connected.to_vec()
        } else {
            Vec::new()
        };
        targets.retain(|id| id != &source && self.monitors.contains_key(id));
        targets.sort_by(|a, b| a.0.cmp(&b.0));
        targets.dedup();
        // Restore the initiating display last so keyboard focus stays there.
        targets.push(source);
        let mut changed = Vec::new();
        for id in targets {
            let monitor = self.monitors.get_mut(&id).expect("target monitor exists");
            while monitor.workspaces.row_count() <= row {
                monitor.workspaces.add_row();
            }
            while monitor.workspaces.column_count() <= column {
                monitor.workspaces.add_column();
            }
            let target = monitor.workspaces.get(row, column).unwrap().id.clone();
            if monitor.active_workspace_id() != &target {
                monitor.set_active_workspace(&target)?;
                changed.push((id, target));
            }
        }
        Ok(changed)
    }

    /// Ensure each connected display has a grid. Never choose a source automatically.
    /// Return newly connected displays for the infrastructure to prompt, if enabled.
    pub fn reconcile_monitors(&mut self, mut connected: Vec<MonitorId>) -> Vec<MonitorId> {
        connected.sort_by(|a, b| a.0.cmp(&b.0));
        connected.dedup();
        let added = connected
            .iter()
            .cloned()
            .filter(|id| !self.connected_monitors.contains(id))
            .collect();
        for target in &connected {
            self.monitors
                .entry(target.clone())
                .or_insert_with(VirtualDesktopMonitor::create);
        }
        self.connected_monitors = connected;
        added
    }

    pub fn reusable_monitor_grids(&self, connected: &[MonitorId]) -> Vec<MonitorId> {
        let mut sources: Vec<_> = self
            .monitors
            .keys()
            .filter(|id| !connected.contains(id))
            .cloned()
            .collect();
        sources.sort_by(|a, b| a.0.cmp(&b.0));
        sources
    }

    /// Explicitly selected reuse: swap grids without duplicating workspace IDs.
    /// Keep currently visible windows on the target's adopted active workspace.
    /// The target's previous grid remains saved under the disconnected source ID.
    pub fn reuse_monitor_grid(
        &mut self,
        target: &MonitorId,
        source: &MonitorId,
    ) -> Result<Vec<isize>> {
        if target == source
            || !self.monitors.contains_key(target)
            || !self.monitors.contains_key(source)
            || !self.connected_monitors.contains(target)
            || self.connected_monitors.contains(source)
        {
            return Err("Invalid workspace grid selection".into());
        }
        let mut adopted = self.monitors.remove(source).unwrap();
        let mut previous = self.monitors.remove(target).unwrap();
        let windows = std::mem::take(&mut previous.active_workspace_mut().windows);
        adopted
            .active_workspace_mut()
            .windows
            .extend(windows.iter().copied());
        self.monitors.insert(target.clone(), adopted);
        self.monitors.insert(source.clone(), previous);
        Ok(windows)
    }

    pub fn sanitize(&mut self) {
        let mut seen = HashSet::new();
        self.pinned.retain(|x| seen.insert(*x));

        for monitor in self.monitors.values_mut() {
            monitor.sanitize();
            /* for workspace in &mut monitor.workspaces {
                workspace.windows.retain(|x| seen.insert(*x));
            } */
            for row in &mut monitor.workspaces.0 {
                for workspace in row {
                    workspace.windows.retain(|x| seen.insert(*x));
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
pub struct VirtualDesktopMonitor {
    pub workspaces: WorkspaceGrid,
    active_workspace: WorkspaceId,
}

impl VirtualDesktopMonitor {
    pub fn create() -> Self {
        let workspace = DesktopWorkspace::create();
        let current_workspace = workspace.id.clone();
        Self {
            workspaces: WorkspaceGrid(vec![vec![workspace]]),
            active_workspace: current_workspace,
        }
    }

    pub fn sanitize(&mut self) {
        self.workspaces.sanitize();

        if !self.workspaces.contains(&self.active_workspace) {
            self.active_workspace = self.workspaces.0[0][0].id.clone();
        }
    }

    pub fn active_workspace_id(&self) -> &WorkspaceId {
        &self.active_workspace
    }

    pub fn active_row(&self) -> &Vec<DesktopWorkspace> {
        for row in &self.workspaces.0 {
            if row.iter().any(|x| x.id == self.active_workspace) {
                return row;
            }
        }
        panic!("Active workspace doesn't exist in the grid");
    }

    pub fn active_workspace(&self) -> &DesktopWorkspace {
        self.workspaces
            .get_by_id(&self.active_workspace)
            .expect("current workspace not found")
    }

    pub fn active_workspace_mut(&mut self) -> &mut DesktopWorkspace {
        self.workspaces
            .get_by_id_mut(&self.active_workspace)
            .expect("current workspace not found")
    }

    /// Set the current workspace, return error if the workspace doesn't exist
    pub fn set_active_workspace(&mut self, workspace_id: &WorkspaceId) -> Result<()> {
        if self.workspaces.contains(workspace_id) {
            self.active_workspace = workspace_id.clone();
            Ok(())
        } else {
            Err("Invalid workspace id".into())
        }
    }

    /// Add a new workspace column and return the id of the new cell on the active row.
    /// The active workspace is not changed, switching to it is up to the caller.
    pub fn add_workspace_column(&mut self) -> WorkspaceId {
        self.workspaces.add_column();
        let row = self.active_row();
        row.last().unwrap().id.clone()
    }

    /// Add a new workspace row and return the id of the new cell on the active column.
    /// The active workspace is not changed, switching to it is up to the caller.
    pub fn add_workspace_row(&mut self) -> WorkspaceId {
        let (_, col_idx) = self
            .workspaces
            .position(&self.active_workspace)
            .expect("Active workspace doesn't exist in the grid");
        self.workspaces.add_row();
        self.workspaces.0.last().unwrap()[col_idx].id.clone()
    }

    /// Remove a workspace by id
    /// - If the grid has more than one column, removes the whole column across every row
    ///   (minimum 1 column required), moving all windows from each removed cell to the side
    ///   column in its own row
    /// - Otherwise, if the grid has more than one row, removes the current row instead,
    ///   moving all windows from the removed cell to the side row in the same column
    /// - If the removed workspace was current, switches to the side one
    /// - Fails if the grid is already 1x1
    pub fn remove_workspace(&mut self, workspace_id: &WorkspaceId) -> Result<()> {
        if self.workspaces.column_count() <= 1 {
            return self.remove_workspace_row(workspace_id);
        }

        // Find the column to remove
        let (_, col_idx) = self
            .workspaces
            .position(workspace_id)
            .ok_or("Workspace not found")?;

        let col_to_move = if col_idx == 0 { 1 } else { col_idx - 1 };

        let mut new_active = None;
        for row in &mut self.workspaces.0 {
            let windows = row[col_idx].windows.clone();
            row[col_to_move].windows.extend(windows);

            if row[col_idx].id == self.active_workspace {
                new_active = Some(row[col_to_move].id.clone());
            }

            row.remove(col_idx);
        }

        if let Some(active) = new_active {
            self.active_workspace = active;
        }

        self.workspaces.sanitize();
        Ok(())
    }

    /// Remove the row containing `workspace_id`, moving its windows to the side row in the
    /// same column. Fails if it's the last remaining row.
    fn remove_workspace_row(&mut self, workspace_id: &WorkspaceId) -> Result<()> {
        if self.workspaces.row_count() <= 1 {
            return Err("Cannot remove the last workspace".into());
        }

        let (row_idx, _) = self
            .workspaces
            .position(workspace_id)
            .ok_or("Workspace not found")?;

        let row_to_move = if row_idx == 0 { 1 } else { row_idx - 1 };

        let mut new_active = None;
        for col_idx in 0..self.workspaces.column_count() {
            let windows = self.workspaces.0[row_idx][col_idx].windows.clone();
            self.workspaces.0[row_to_move][col_idx]
                .windows
                .extend(windows);

            if self.workspaces.0[row_idx][col_idx].id == self.active_workspace {
                new_active = Some(self.workspaces.0[row_to_move][col_idx].id.clone());
            }
        }

        self.workspaces.0.remove(row_idx);

        if let Some(active) = new_active {
            self.active_workspace = active;
        }

        self.workspaces.sanitize();
        Ok(())
    }

    /// Rename a workspace by id
    pub fn rename_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
        name: Option<String>,
    ) -> Result<()> {
        let workspace = self
            .workspaces
            .get_by_id_mut(workspace_id)
            .ok_or("Workspace not found")?;
        workspace.name = name;
        Ok(())
    }

    pub fn set_wallpapers(&mut self, f: impl Fn(&DesktopWorkspace) -> Option<WallpaperId>) {
        for row in &mut self.workspaces.0 {
            for workspace in row {
                workspace.wallpaper = f(workspace);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
pub struct WorkspaceGrid(Vec<Vec<DesktopWorkspace>>);

impl WorkspaceGrid {
    pub fn rows(&self) -> &Vec<Vec<DesktopWorkspace>> {
        &self.0
    }

    pub fn rows_mut(&mut self) -> &mut Vec<Vec<DesktopWorkspace>> {
        &mut self.0
    }

    pub fn row_count(&self) -> usize {
        self.0.len()
    }

    pub fn column_count(&self) -> usize {
        self.0[0].len()
    }

    /// Ensures the grid has at least one row/column (1x1 minimum) and that
    /// every row has the same number of columns, padding short rows with new workspaces.
    pub fn sanitize(&mut self) {
        self.0.retain(|row| !row.is_empty());

        if self.0.is_empty() {
            self.0.push(vec![DesktopWorkspace::create()]);
            return;
        }

        let max_columns = self.0.iter().map(|row| row.len()).max().unwrap_or(1).max(1);
        for row in &mut self.0 {
            while row.len() < max_columns {
                row.push(DesktopWorkspace::create());
            }
        }
    }

    /// Find the (row, column) position of a workspace by id
    pub fn position(&self, workspace_id: &WorkspaceId) -> Option<(usize, usize)> {
        for (row_idx, row) in self.0.iter().enumerate() {
            for (col_idx, workspace) in row.iter().enumerate() {
                if &workspace.id == workspace_id {
                    return Some((row_idx, col_idx));
                }
            }
        }
        None
    }

    pub fn add_row(&mut self) {
        let mut row = Vec::new();
        for _ in 0..self.column_count() {
            row.push(DesktopWorkspace::create());
        }
        self.0.push(row);
    }

    pub fn add_column(&mut self) {
        for row in &mut self.0 {
            row.push(DesktopWorkspace::create());
        }
    }

    pub fn contains(&self, workspace_id: &WorkspaceId) -> bool {
        for row in &self.0 {
            for workspace in row {
                if &workspace.id == workspace_id {
                    return true;
                }
            }
        }
        false
    }

    pub fn get(&self, row: usize, col: usize) -> Option<&DesktopWorkspace> {
        self.0.get(row)?.get(col)
    }

    pub fn get_by_window_id(&self, window_id: isize) -> Option<&DesktopWorkspace> {
        for row in &self.0 {
            for workspace in row {
                if workspace.windows.contains(&window_id) {
                    return Some(workspace);
                }
            }
        }
        None
    }

    pub fn get_by_window_id_mut(&mut self, window_id: isize) -> Option<&mut DesktopWorkspace> {
        for row in &mut self.0 {
            for workspace in row {
                if workspace.windows.contains(&window_id) {
                    return Some(workspace);
                }
            }
        }
        None
    }

    pub fn get_by_id(&self, workspace_id: &WorkspaceId) -> Option<&DesktopWorkspace> {
        for row in &self.0 {
            for workspace in row {
                if &workspace.id == workspace_id {
                    return Some(workspace);
                }
            }
        }
        None
    }

    pub fn get_by_id_mut(&mut self, workspace_id: &WorkspaceId) -> Option<&mut DesktopWorkspace> {
        for row in &mut self.0 {
            for workspace in row {
                if &workspace.id == workspace_id {
                    return Some(workspace);
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
pub struct DesktopWorkspace {
    pub id: WorkspaceId,
    pub name: Option<String>,
    /// react-icon icon name
    pub icon: Option<String>,
    pub wallpaper: Option<WallpaperId>,
    #[serde(default)]
    pub windows: Vec<isize>,
}

impl DesktopWorkspace {
    pub fn create() -> Self {
        Self {
            id: WorkspaceId(Uuid::new_v4().to_string()),
            name: None,
            icon: None,
            wallpaper: None,
            windows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
pub struct WorkspaceId(pub String);

identifier_impl!(WorkspaceId, String);

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> MonitorId {
        MonitorId(name.to_owned())
    }

    fn home_state() -> VirtualDesktops {
        let mut home = VirtualDesktopMonitor::create();
        let email = home.add_workspace_column();
        home.rename_workspace(&email, Some("Email".into())).unwrap();
        home.set_active_workspace(&email).unwrap();
        home.active_workspace_mut().windows.push(42);
        VirtualDesktops {
            monitors: HashMap::from([(id("home"), home)]),
            connected_monitors: vec![id("home")],
            ..Default::default()
        }
    }

    #[test]
    fn connecting_monitor_never_automatically_reuses_a_grid() {
        let mut state = home_state();
        let home_id = state.monitors[&id("home")].active_workspace_id().clone();
        assert_eq!(state.reconcile_monitors(vec![id("work")]), vec![id("work")]);
        assert_eq!(state.monitors[&id("home")].active_workspace_id(), &home_id);
        assert_ne!(state.monitors[&id("work")].active_workspace_id(), &home_id);
        assert_eq!(state.reusable_monitor_grids(&[id("work")]), vec![id("home")]);
    }

    #[test]
    fn explicit_reuse_keeps_the_selected_grid_and_currently_visible_windows() {
        let mut state = home_state();
        state.monitors.get_mut(&id("home")).unwrap().add_workspace_row();
        let home_id = state.monitors[&id("home")].active_workspace_id().clone();
        state.reconcile_monitors(vec![id("work")]);
        let work = state.monitors.get_mut(&id("work")).unwrap();
        let work_id = work.active_workspace_id().clone();
        work.active_workspace_mut().windows.push(99);
        assert_eq!(state.reuse_monitor_grid(&id("work"), &id("home")).unwrap(), vec![99]);
        let adopted = &state.monitors[&id("work")];
        assert_eq!(adopted.active_workspace_id(), &home_id);
        assert_eq!(adopted.active_workspace().name.as_deref(), Some("Email"));
        assert_eq!(adopted.workspaces.row_count(), 2);
        assert_eq!(adopted.workspaces.column_count(), 2);
        assert_eq!(adopted.active_workspace().windows, vec![42, 99]);
        assert_eq!(state.monitors[&id("home")].active_workspace_id(), &work_id);
        assert!(state.monitors[&id("home")].active_workspace().windows.is_empty());
        let mut ids = HashSet::new();
        for monitor in state.monitors.values() {
            for workspace in monitor.workspaces.rows().iter().flatten() {
                assert!(ids.insert(workspace.id.clone()));
            }
        }
    }

    #[test]
    fn keeping_separate_grids_preserves_existing_monitor_configuration() {
        let mut state = home_state();
        state.reconcile_monitors(vec![id("work")]);
        let work = serde_json::to_value(&state.monitors[&id("work")]).unwrap();
        state.reconcile_monitors(vec![id("home")]);
        state.reconcile_monitors(vec![id("work")]);
        assert_eq!(serde_json::to_value(&state.monitors[&id("work")]).unwrap(), work);
    }

    #[test]
    fn returning_home_requires_an_explicit_choice_and_keeps_latest_edits() {
        let mut state = home_state();
        state.reconcile_monitors(vec![id("work")]);
        state.reuse_monitor_grid(&id("work"), &id("home")).unwrap();
        let added = state.monitors.get_mut(&id("work")).unwrap().add_workspace_column();
        state.reconcile_monitors(vec![id("home")]);
        assert!(!state.monitors[&id("home")].workspaces.contains(&added));
        state.reuse_monitor_grid(&id("home"), &id("work")).unwrap();
        assert!(state.monitors[&id("home")].workspaces.contains(&added));
    }

    #[test]
    fn restarting_with_changed_monitors_offers_saved_disconnected_grids() {
        let mut state = home_state();
        state.monitor_names.insert(id("home"), "Home display".into());
        state.reconcile_monitors(vec![]);
        let mut restarted: VirtualDesktops =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(restarted.reconcile_monitors(vec![id("work")]), vec![id("work")]);
        assert_eq!(restarted.reusable_monitor_grids(&[id("work")]), vec![id("home")]);
        assert_eq!(restarted.monitor_names[&id("home")], "Home display");
    }

    #[test]
    fn repeated_notifications_do_not_repeat_connection_prompts() {
        let mut state = home_state();
        assert_eq!(state.reconcile_monitors(vec![id("work")]), vec![id("work")]);
        assert!(state.reconcile_monitors(vec![id("work")]).is_empty());
        let json = serde_json::to_string(&state).unwrap();
        let mut restarted: VirtualDesktops = serde_json::from_str(&json).unwrap();
        assert!(restarted.reconcile_monitors(vec![id("work")]).is_empty());
    }

    #[test]
    fn connected_monitors_are_not_offered_for_reuse() {
        let mut state = home_state();
        state.monitors.insert(id("old"), VirtualDesktopMonitor::create());
        state.reconcile_monitors(vec![id("home"), id("work")]);
        assert_eq!(state.reusable_monitor_grids(&[id("home"), id("work")]), vec![id("old")]);
    }

    #[test]
    fn invalid_reuse_does_not_change_any_grid() {
        let mut state = home_state();
        let before = serde_json::to_value(&state).unwrap();
        assert!(state.reuse_monitor_grid(&id("home"), &id("home")).is_err());
        assert!(state.reuse_monitor_grid(&id("missing"), &id("home")).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[test]
    fn source_reconnecting_before_choice_cannot_have_its_grid_reused() {
        let mut state = home_state();
        state.reconcile_monitors(vec![id("work")]);
        state.reconcile_monitors(vec![id("home"), id("work")]);
        let before = serde_json::to_value(&state).unwrap();
        assert!(state.reuse_monitor_grid(&id("work"), &id("home")).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), before);
    }

    #[test]
    fn old_cache_files_keep_grids_until_the_user_chooses() {
        let mut json = serde_json::to_value(home_state()).unwrap();
        json.as_object_mut().unwrap().remove("connectedMonitors");
        json.as_object_mut().unwrap().remove("monitorNames");
        let mut state: VirtualDesktops = serde_json::from_value(json).unwrap();
        let home_id = state.monitors[&id("home")].active_workspace_id().clone();
        state.reconcile_monitors(vec![id("work")]);
        assert_eq!(state.monitors[&id("home")].active_workspace_id(), &home_id);
        assert_ne!(state.monitors[&id("work")].active_workspace_id(), &home_id);
    }

    #[test]
    fn synchronized_switch_grows_connected_grids_and_preserves_disconnected_grids() {
        let mut state = home_state();
        let source = state.monitors.get_mut(&id("home")).unwrap();
        let target = source.add_workspace_row();
        state.monitors.insert(id("laptop"), VirtualDesktopMonitor::create());
        state.monitors.insert(id("offline"), VirtualDesktopMonitor::create());
        let offline = serde_json::to_value(&state.monitors[&id("offline")]).unwrap();
        let changed = state
            .switch_to_workspace(&target, &[id("home"), id("laptop")], true)
            .unwrap();
        assert_eq!(changed.last(), Some(&(id("home"), target)));
        let laptop = &state.monitors[&id("laptop")];
        assert_eq!(laptop.workspaces.position(laptop.active_workspace_id()), Some((1, 1)));
        assert_eq!(laptop.workspaces.row_count(), 2);
        assert_eq!(laptop.workspaces.column_count(), 2);
        assert_eq!(serde_json::to_value(&state.monitors[&id("offline")]).unwrap(), offline);
        // Each monitor has its own IDs and windows even when their positions match.
        assert_ne!(laptop.active_workspace_id(), state.monitors[&id("home")].active_workspace_id());
        assert!(laptop.active_workspace().windows.is_empty());
    }

    #[test]
    fn independent_switch_does_not_change_other_monitors() {
        let mut state = home_state();
        let target = state.monitors[&id("home")].workspaces.get(0, 0).unwrap().id.clone();
        state.monitors.insert(id("laptop"), VirtualDesktopMonitor::create());
        let laptop = serde_json::to_value(&state.monitors[&id("laptop")]).unwrap();
        let changed = state
            .switch_to_workspace(&target, &[id("home"), id("laptop")], false)
            .unwrap();
        assert_eq!(changed, vec![(id("home"), target)]);
        assert_eq!(serde_json::to_value(&state.monitors[&id("laptop")]).unwrap(), laptop);
    }

    #[test]
    fn synchronized_switch_aligns_other_monitors_even_when_source_is_already_active() {
        let mut state = home_state();
        let target = state.monitors[&id("home")].active_workspace_id().clone();
        state.monitors.insert(id("laptop"), VirtualDesktopMonitor::create());
        let connected = [id("home"), id("laptop")];
        let changed = state.switch_to_workspace(&target, &connected, true).unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0, id("laptop"));
        assert!(state.switch_to_workspace(&target, &connected, true).unwrap().is_empty());
    }

    #[test]
    fn synchronized_switch_and_portability_can_be_combined() {
        let mut state = home_state();
        state.monitors.insert(id("laptop"), VirtualDesktopMonitor::create());
        state.connected_monitors.push(id("laptop"));
        let target = state.monitors[&id("home")].active_workspace_id().clone();
        state.reconcile_monitors(vec![id("work"), id("laptop")]);
        state.reuse_monitor_grid(&id("work"), &id("home")).unwrap();
        state.switch_to_workspace(&target, &[id("work"), id("laptop")], true).unwrap();
        for monitor_id in [id("work"), id("laptop")] {
            let monitor = &state.monitors[&monitor_id];
            assert_eq!(monitor.workspaces.position(monitor.active_workspace_id()), Some((0, 1)));
        }
        assert_eq!(state.monitors[&id("work")].active_workspace().windows, vec![42]);
    }

    #[test]
    fn invalid_switch_leaves_every_monitor_unchanged() {
        let mut state = home_state();
        let snapshot = serde_json::to_value(&state).unwrap();
        assert!(state.switch_to_workspace(&WorkspaceId("missing".into()), &[id("home")], true).is_err());
        assert_eq!(serde_json::to_value(&state).unwrap(), snapshot);
    }
}
