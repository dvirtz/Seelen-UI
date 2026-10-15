use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use uuid::Uuid;

use crate::{error::Result, identifier_impl, resource::WallpaperId, system_state::MonitorId};

#[derive(Debug, Default, Clone, Serialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), ts(export))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(rename = "WorkspaceState")
)]
pub struct VirtualDesktops {
    pub monitors: HashMap<MonitorId, VirtualDesktopMonitor>,
    /// Authoritative definitions and window membership, stored once per workspace ID.
    pub workspaces: HashMap<WorkspaceId, DesktopWorkspace>,
    pub pinned: Vec<isize>,
    pub switching: bool,
    pub connected_monitors: Vec<MonitorId>,
    pub monitor_names: HashMap<MonitorId, String>,
}

impl VirtualDesktops {
    pub fn ensure_monitor(&mut self, id: MonitorId) {
        if !self.monitors.contains_key(&id) {
            let workspace = self.new_workspace();
            self.monitors
                .insert(id, VirtualDesktopMonitor::create(workspace));
        }
    }

    fn new_workspace(&mut self) -> WorkspaceId {
        let workspace = DesktopWorkspace::create();
        let id = workspace.id.clone();
        self.workspaces.insert(id.clone(), workspace);
        id
    }

    pub fn reconcile_monitors(&mut self, mut connected: Vec<MonitorId>) -> Vec<MonitorId> {
        connected.sort_by(|a, b| a.0.cmp(&b.0));
        connected.dedup();
        let added = connected
            .iter()
            .filter(|id| !self.connected_monitors.contains(id))
            .cloned()
            .collect();
        for id in &connected {
            self.ensure_monitor(id.clone());
        }
        let mut priority = connected.clone();
        priority.sort_by(|a, b| {
            (!self.connected_monitors.contains(a), &a.0)
                .cmp(&(!self.connected_monitors.contains(b), &b.0))
        });
        let mut aligned = Vec::new();
        for id in priority {
            let source = self.monitors[&id].clone();
            if aligned.contains(&source.workspaces) {
                continue;
            }
            for monitor in self.monitors.values_mut() {
                if monitor.workspaces == source.workspaces {
                    monitor.active_workspace = source.active_workspace.clone();
                }
            }
            aligned.push(source.workspaces);
        }
        self.connected_monitors = connected;
        added
    }

    pub fn monitors_for_workspace(&self, id: &WorkspaceId) -> Vec<MonitorId> {
        let mut ids: Vec<_> = self
            .monitors
            .iter()
            .filter(|(_, monitor)| monitor.workspaces.contains(id))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort_by(|a, b| {
            (!self.connected_monitors.contains(a), &a.0)
                .cmp(&(!self.connected_monitors.contains(b), &b.0))
        });
        ids
    }

    pub fn active_workspace_ids(&self) -> HashSet<WorkspaceId> {
        self.connected_monitors
            .iter()
            .filter_map(|id| self.monitors.get(id))
            .map(|monitor| monitor.active_workspace.clone())
            .collect()
    }

    pub fn reusable_monitor_grids(&self, connected: &[MonitorId]) -> Vec<MonitorId> {
        let mut ids: Vec<_> = self
            .monitors
            .keys()
            .filter(|id| !connected.contains(id))
            .cloned()
            .collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        ids
    }

    /// Link both monitors to the same IDs; the source's references are never swapped.
    pub fn reuse_monitor_grid(
        &mut self,
        target: &MonitorId,
        source: &MonitorId,
    ) -> Result<Vec<isize>> {
        if target == source
            || !self.connected_monitors.contains(target)
            || self.connected_monitors.contains(source)
        {
            return Err("Invalid workspace grid selection".into());
        }
        let adopted = self
            .monitors
            .get(source)
            .ok_or("Source monitor not found")?
            .clone();
        let previous = self
            .monitors
            .get(target)
            .ok_or("Target monitor not found")?
            .clone();
        if !self.workspaces.contains_key(adopted.active_workspace_id()) {
            return Err("Active workspace not found".into());
        }
        let mut moved = Vec::new();
        // Preserve windows from every displaced cell, including hidden cells.
        for id in previous.workspaces.rows().iter().flatten() {
            if !adopted.workspaces.contains(id)
                && self
                    .monitors_for_workspace(id)
                    .iter()
                    .all(|owner| owner == target)
            {
                if let Some(workspace) = self.workspaces.get_mut(id) {
                    moved.append(&mut workspace.windows);
                }
            }
        }
        self.workspaces
            .get_mut(adopted.active_workspace_id())
            .ok_or("Active workspace not found")?
            .windows
            .extend(moved.iter().copied());
        self.monitors.insert(target.clone(), adopted);
        Ok(moved)
    }

    /// Apply grid edits to every monitor referencing that grid, including offline monitors.
    fn replace_grid(&mut self, old: &WorkspaceGrid, new: WorkspaceGrid) {
        for monitor in self.monitors.values_mut() {
            if &monitor.workspaces == old {
                monitor.workspaces = new.clone();
                if !new.contains(&monitor.active_workspace) {
                    monitor.active_workspace = new.rows()[0][0].clone();
                }
            }
        }
    }

    fn grow_grid(&mut self, monitor_id: &MonitorId, rows: usize, columns: usize) -> Result<()> {
        let old = self
            .monitors
            .get(monitor_id)
            .ok_or("Monitor not found")?
            .workspaces
            .clone();
        let mut grid = old.clone();
        while grid.row_count() < rows {
            grid.0.push(
                (0..grid.column_count())
                    .map(|_| self.new_workspace())
                    .collect(),
            );
        }
        for row in &mut grid.0 {
            while row.len() < columns {
                row.push(self.new_workspace());
            }
        }
        self.replace_grid(&old, grid);
        Ok(())
    }

    pub fn create_workspace(
        &mut self,
        monitor_id: &MonitorId,
        as_row: bool,
    ) -> Result<WorkspaceId> {
        let monitor = self.monitors.get(monitor_id).ok_or("Monitor not found")?;
        let (row, column) = monitor
            .workspaces
            .position(monitor.active_workspace_id())
            .ok_or("Active workspace not found")?;
        let rows = monitor.workspaces.row_count();
        let columns = monitor.workspaces.column_count();
        self.grow_grid(
            monitor_id,
            rows + usize::from(as_row),
            columns + usize::from(!as_row),
        )?;
        Ok(self.monitors[monitor_id]
            .workspaces
            .get(
                if as_row { rows } else { row },
                if as_row { column } else { columns },
            )
            .unwrap()
            .clone())
    }

    pub fn switch_to_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
        connected: &[MonitorId],
        together: bool,
    ) -> Result<Vec<(MonitorId, WorkspaceId)>> {
        let source = self
            .monitors_for_workspace(workspace_id)
            .into_iter()
            .find(|id| connected.contains(id))
            .ok_or("Workspace has no connected monitor")?;
        let original = self.monitors[&source].workspaces.clone();
        let (row, column) = original
            .position(workspace_id)
            .ok_or("Workspace not found")?;
        let mut targets: Vec<_> = self
            .monitors
            .iter()
            .filter(|(id, monitor)| {
                monitor.workspaces == original || (together && connected.contains(id))
            })
            .map(|(id, _)| id.clone())
            .collect();
        targets.sort_by(|a, b| a.0.cmp(&b.0));
        targets.retain(|id| id != &source);
        targets.push(source);
        let mut changed = Vec::new();
        for id in targets {
            self.grow_grid(&id, row + 1, column + 1)?;
            let target = self.monitors[&id]
                .workspaces
                .get(row, column)
                .unwrap()
                .clone();
            let grid = self.monitors[&id].workspaces.clone();
            for (peer_id, monitor) in &mut self.monitors {
                if monitor.workspaces == grid && monitor.active_workspace != target {
                    monitor.active_workspace = target.clone();
                    if connected.contains(peer_id) {
                        changed.push((peer_id.clone(), target.clone()));
                    }
                }
            }
        }
        changed.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
        Ok(changed)
    }

    pub fn remove_workspace(&mut self, id: &WorkspaceId) -> Result<Vec<WorkspaceId>> {
        let source = self
            .monitors_for_workspace(id)
            .into_iter()
            .next()
            .ok_or("Workspace not found")?;
        let old = self.monitors[&source].workspaces.clone();
        let (row, column) = old.position(id).ok_or("Workspace not found")?;
        if old.row_count() == 1 && old.column_count() == 1 {
            return Err("Cannot remove the last workspace".into());
        }
        let remove_column = old.column_count() > 1;
        let mut grid = old.clone();
        let mut redirects = Vec::new();
        if remove_column {
            let side = if column == 0 { 1 } else { column - 1 };
            for cells in &mut grid.0 {
                redirects.push((cells[column].clone(), cells[side].clone()));
                cells.remove(column);
            }
        } else {
            let side = if row == 0 { 1 } else { row - 1 };
            for column in 0..grid.column_count() {
                redirects.push((grid.0[row][column].clone(), grid.0[side][column].clone()));
            }
            grid.0.remove(row);
        }
        for monitor in self.monitors.values_mut() {
            if monitor.workspaces == old {
                if let Some((_, destination)) = redirects
                    .iter()
                    .find(|(removed, _)| removed == &monitor.active_workspace)
                {
                    monitor.active_workspace = destination.clone();
                }
            }
        }
        self.replace_grid(&old, grid);
        let mut removed_ids = Vec::new();
        for (removed, destination) in redirects {
            if !self.monitors_for_workspace(&removed).is_empty() {
                continue;
            }
            if let Some(workspace) = self.workspaces.remove(&removed) {
                self.workspaces
                    .get_mut(&destination)
                    .ok_or("Destination workspace not found")?
                    .windows
                    .extend(workspace.windows);
                removed_ids.push(removed);
            }
        }
        Ok(removed_ids)
    }

    pub fn sanitize(&mut self) {
        let mut ids: Vec<_> = self.monitors.keys().cloned().collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        for id in ids {
            let mut grid = self.monitors[&id].workspaces.clone();
            grid.0.retain(|row| !row.is_empty());
            if grid.0.is_empty() {
                grid.0.push(vec![self.new_workspace()]);
            }
            let columns = grid.0.iter().map(Vec::len).max().unwrap_or(1);
            let mut unique = HashSet::new();
            for row in &mut grid.0 {
                for workspace_id in row.iter_mut() {
                    if !unique.insert(workspace_id.clone()) {
                        *workspace_id = self.new_workspace();
                        unique.insert(workspace_id.clone());
                    }
                    self.workspaces
                        .entry(workspace_id.clone())
                        .or_insert_with(|| {
                            let mut workspace = DesktopWorkspace::create();
                            workspace.id = workspace_id.clone();
                            workspace
                        });
                }
                while row.len() < columns {
                    row.push(self.new_workspace());
                }
            }
            let monitor = self.monitors.get_mut(&id).unwrap();
            monitor.workspaces = grid;
            if !monitor.workspaces.contains(&monitor.active_workspace) {
                monitor.active_workspace = monitor.workspaces.rows()[0][0].clone();
            }
        }
        for (id, workspace) in &mut self.workspaces {
            workspace.id = id.clone();
        }
        let mut seen = HashSet::new();
        self.pinned.retain(|window| seen.insert(*window));
        let mut ids: Vec<_> = self.workspaces.keys().cloned().collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        for id in ids {
            self.workspaces
                .get_mut(&id)
                .unwrap()
                .windows
                .retain(|window| seen.insert(*window));
        }
        self.connected_monitors.sort_by(|a, b| a.0.cmp(&b.0));
        self.connected_monitors.dedup();
    }
}

impl<'de> Deserialize<'de> for VirtualDesktops {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let stored = StoredVirtualDesktops::deserialize(deserializer)?;
        let mut state = Self {
            workspaces: stored.workspaces,
            pinned: stored.pinned,
            switching: stored.switching,
            connected_monitors: stored.connected_monitors,
            monitor_names: stored.monitor_names,
            ..Default::default()
        };
        let mut monitors: Vec<_> = stored.monitors.into_iter().collect();
        monitors.sort_by(|(a, _), (b, _)| a.0.cmp(&b.0));
        for (id, monitor) in monitors {
            let mut rows = Vec::new();
            for row in monitor.workspaces {
                let mut ids = Vec::new();
                for entry in row {
                    let workspace_id = match entry {
                        StoredWorkspace::Id(id) => id,
                        StoredWorkspace::Definition(workspace) => {
                            let id = workspace.id.clone();
                            if let Some(existing) = state.workspaces.get_mut(&id) {
                                existing.windows.extend(workspace.windows);
                            } else {
                                state.workspaces.insert(id.clone(), workspace);
                            }
                            id
                        }
                    };
                    ids.push(workspace_id);
                }
                rows.push(ids);
            }
            state.monitors.insert(
                id,
                VirtualDesktopMonitor {
                    workspaces: WorkspaceGrid(rows),
                    active_workspace: monitor.active_workspace,
                },
            );
        }
        state.sanitize();
        Ok(state)
    }
}

/// Public API projection. Keep the legacy definition grids out of persisted state.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(export, rename = "VirtualDesktops")
)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDesktopsView {
    pub monitors: HashMap<MonitorId, VirtualDesktopMonitorView>,
    pub workspaces: HashMap<WorkspaceId, DesktopWorkspace>,
    pub pinned: Vec<isize>,
    pub switching: bool,
    pub connected_monitors: Vec<MonitorId>,
    pub monitor_names: HashMap<MonitorId, String>,
}

impl From<VirtualDesktops> for VirtualDesktopsView {
    fn from(state: VirtualDesktops) -> Self {
        let monitors = state
            .monitors
            .into_iter()
            .map(|(id, monitor)| {
                let workspaces = monitor
                    .workspaces
                    .rows()
                    .iter()
                    .map(|row| {
                        row.iter()
                            .filter_map(|id| state.workspaces.get(id).cloned())
                            .collect()
                    })
                    .collect();
                (
                    id,
                    VirtualDesktopMonitorView {
                        workspaces: WorkspaceDefinitionGrid(workspaces),
                        workspace_ids: monitor.workspaces,
                        active_workspace: monitor.active_workspace,
                    },
                )
            })
            .collect();
        Self {
            monitors,
            workspaces: state.workspaces,
            pinned: state.pinned,
            switching: state.switching,
            connected_monitors: state.connected_monitors,
            monitor_names: state.monitor_names,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(export, rename = "VirtualDesktopMonitor")
)]
pub struct VirtualDesktopMonitorView {
    /// Computed definitions for existing widgets; never another source of state.
    pub workspaces: WorkspaceDefinitionGrid,
    #[serde(rename = "workspaceIds")]
    pub workspace_ids: WorkspaceGrid,
    pub active_workspace: WorkspaceId,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(export, rename = "WorkspaceGrid")
)]
pub struct WorkspaceDefinitionGrid(Vec<Vec<DesktopWorkspace>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(rename = "WorkspaceMonitor")
)]
pub struct VirtualDesktopMonitor {
    pub workspaces: WorkspaceGrid,
    active_workspace: WorkspaceId,
}

impl VirtualDesktopMonitor {
    pub fn create(workspace: WorkspaceId) -> Self {
        Self {
            workspaces: WorkspaceGrid(vec![vec![workspace.clone()]]),
            active_workspace: workspace,
        }
    }
    pub fn active_workspace_id(&self) -> &WorkspaceId {
        &self.active_workspace
    }
    pub fn active_row(&self) -> &Vec<WorkspaceId> {
        &self.workspaces.rows()[self.workspaces.position(&self.active_workspace).unwrap().0]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
#[cfg_attr(
    all(feature = "gen-binds", not(feature = "salvo")),
    ts(rename = "WorkspaceIdGrid")
)]
pub struct WorkspaceGrid(Vec<Vec<WorkspaceId>>);

impl WorkspaceGrid {
    pub fn rows(&self) -> &Vec<Vec<WorkspaceId>> {
        &self.0
    }
    pub fn row_count(&self) -> usize {
        self.0.len()
    }
    pub fn column_count(&self) -> usize {
        self.0.first().map_or(0, Vec::len)
    }
    pub fn contains(&self, id: &WorkspaceId) -> bool {
        self.position(id).is_some()
    }
    pub fn get(&self, row: usize, column: usize) -> Option<&WorkspaceId> {
        self.0.get(row)?.get(column)
    }
    pub fn position(&self, id: &WorkspaceId) -> Option<(usize, usize)> {
        self.0.iter().enumerate().find_map(|(row, cells)| {
            cells
                .iter()
                .position(|cell| cell == id)
                .map(|column| (row, column))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(all(feature = "gen-binds", not(feature = "salvo")), derive(ts_rs::TS))]
pub struct DesktopWorkspace {
    pub id: WorkspaceId,
    pub name: Option<String>,
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

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StoredVirtualDesktops {
    monitors: HashMap<MonitorId, StoredMonitor>,
    workspaces: HashMap<WorkspaceId, DesktopWorkspace>,
    pinned: Vec<isize>,
    switching: bool,
    connected_monitors: Vec<MonitorId>,
    monitor_names: HashMap<MonitorId, String>,
}
#[derive(Deserialize)]
struct StoredMonitor {
    workspaces: Vec<Vec<StoredWorkspace>>,
    active_workspace: WorkspaceId,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredWorkspace {
    Id(WorkspaceId),
    Definition(DesktopWorkspace),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(id: &str) -> MonitorId {
        MonitorId(id.into())
    }
    fn shared() -> VirtualDesktops {
        let mut state = VirtualDesktops::default();
        state.reconcile_monitors(vec![monitor("home")]);
        state.create_workspace(&monitor("home"), false).unwrap();
        state.reconcile_monitors(vec![monitor("work")]);
        state
            .reuse_monitor_grid(&monitor("work"), &monitor("home"))
            .unwrap();
        state
    }
    #[test]
    fn sharing_references_one_definition_and_one_window_set() {
        let mut state = shared();
        assert_eq!(
            state.monitors[&monitor("home")].workspaces,
            state.monitors[&monitor("work")].workspaces
        );
        let id = state.monitors[&monitor("home")]
            .active_workspace_id()
            .clone();
        state.workspaces.get_mut(&id).unwrap().name = Some("Coding".into());
        state.workspaces.get_mut(&id).unwrap().windows.push(42);
        let json = serde_json::to_value(&state).unwrap();
        assert!(json["monitors"]["home"]["workspaces"][0][0].is_string());
        assert_eq!(
            json["workspaces"][&id.0]["windows"],
            serde_json::json!([42])
        );
        let restored: VirtualDesktops = serde_json::from_value(json).unwrap();
        assert_eq!(restored.workspaces[&id].name.as_deref(), Some("Coding"));
        assert_eq!(restored.workspaces[&id].windows, vec![42]);
    }
    #[test]
    fn api_preserves_legacy_definition_grids_without_denormalizing_storage() {
        let mut state = shared();
        let id = state.monitors[&monitor("home")]
            .active_workspace_id()
            .clone();
        state.workspaces.get_mut(&id).unwrap().name = Some("Coding".into());
        state.workspaces.get_mut(&id).unwrap().windows = vec![42];
        let view = serde_json::to_value(VirtualDesktopsView::from(state.clone())).unwrap();
        for owner in ["home", "work"] {
            let monitor = &view["monitors"][owner];
            assert_eq!(monitor["active_workspace"], id.0);
            assert_eq!(monitor["workspaceIds"][0][0], id.0);
            assert_eq!(monitor["workspaces"][0][0], view["workspaces"][&id.0]);
            assert_eq!(monitor["workspaces"][0][0]["name"], "Coding");
            assert_eq!(
                monitor["workspaces"][0][0]["windows"],
                serde_json::json!([42])
            );
        }
        // Derive each response from the current registry, without cached copies.
        state.workspaces.get_mut(&id).unwrap().name = Some("Updated".into());
        let updated = serde_json::to_value(VirtualDesktopsView::from(state.clone())).unwrap();
        assert_eq!(
            updated["monitors"]["work"]["workspaces"][0][0]["name"],
            "Updated"
        );
        let stored = serde_json::to_value(state).unwrap();
        assert_eq!(stored["monitors"]["home"]["workspaces"][0][0], id.0);
        assert!(stored["monitors"]["home"].get("workspaceIds").is_none());
    }
    #[test]
    fn edits_propagate_to_offline_references() {
        let mut state = shared();
        let id = state.create_workspace(&monitor("work"), true).unwrap();
        assert!(state.monitors[&monitor("home")].workspaces.contains(&id));
        state.remove_workspace(&id).unwrap();
        assert_eq!(
            state.monitors[&monitor("home")].workspaces,
            state.monitors[&monitor("work")].workspaces
        );
    }
    #[test]
    fn shared_grids_switch_together_without_synchronizing_other_grids() {
        let mut state = shared();
        state.reconcile_monitors(vec![monitor("home"), monitor("work"), monitor("other")]);
        let other = state.monitors[&monitor("other")]
            .active_workspace_id()
            .clone();
        let id = state.monitors[&monitor("work")]
            .workspaces
            .get(0, 1)
            .unwrap()
            .clone();
        state
            .switch_to_workspace(&id, &state.connected_monitors.clone(), false)
            .unwrap();
        assert_eq!(state.monitors[&monitor("home")].active_workspace_id(), &id);
        assert_eq!(state.monitors[&monitor("work")].active_workspace_id(), &id);
        assert_eq!(
            state.monitors[&monitor("other")].active_workspace_id(),
            &other
        );
        state
            .switch_to_workspace(&id, &state.connected_monitors.clone(), true)
            .unwrap();
        let other = &state.monitors[&monitor("other")];
        assert_eq!(
            other.workspaces.position(other.active_workspace_id()),
            Some((0, 1))
        );
    }
    #[test]
    fn linking_preserves_windows_in_displaced_hidden_cells() {
        let mut state = shared();
        state.reconcile_monitors(vec![monitor("new")]);
        let hidden = state.create_workspace(&monitor("new"), false).unwrap();
        state.workspaces.get_mut(&hidden).unwrap().windows.push(77);
        let moved = state
            .reuse_monitor_grid(&monitor("new"), &monitor("home"))
            .unwrap();
        assert_eq!(moved, vec![77]);
        let id = state.monitors[&monitor("new")].active_workspace_id();
        assert!(state.workspaces[id].windows.contains(&77));
    }
    #[test]
    fn relinking_one_monitor_preserves_another_grids_shared_windows() {
        let mut state = shared();
        let old = state.monitors[&monitor("work")]
            .active_workspace_id()
            .clone();
        state.workspaces.get_mut(&old).unwrap().windows.push(42);
        state.ensure_monitor(monitor("other"));
        let moved = state
            .reuse_monitor_grid(&monitor("work"), &monitor("other"))
            .unwrap();
        assert!(moved.is_empty());
        assert_eq!(state.workspaces[&old].windows, vec![42]);
        assert!(state.monitors[&monitor("home")].workspaces.contains(&old));
        assert!(!state.monitors[&monitor("work")].workspaces.contains(&old));
    }

    #[test]
    fn old_nested_files_are_migrated_without_changing_ids_or_windows() {
        let id = WorkspaceId("existing".into());
        let mut definition = DesktopWorkspace::create();
        definition.id = id.clone();
        definition.name = Some("Email".into());
        definition.windows = vec![42, 42];
        let json = serde_json::json!({
            "monitors": { "home": { "workspaces": [[definition.clone()]], "active_workspace": id },
                          "work": { "workspaces": [[definition]], "active_workspace": id } }
        });
        let state: VirtualDesktops = serde_json::from_value(json).unwrap();
        assert_eq!(state.workspaces.len(), 1);
        assert_eq!(state.workspaces[&id].windows, vec![42]);
        assert_eq!(state.workspaces[&id].name.as_deref(), Some("Email"));
    }
    #[test]
    fn dangling_references_and_empty_grids_are_repaired() {
        let json = serde_json::json!({ "monitors": {
            "home": { "workspaces": [], "active_workspace": "missing" },
            "work": { "workspaces": [["known", "known"]], "active_workspace": "missing" }
        }});
        let state: VirtualDesktops = serde_json::from_value(json).unwrap();
        for monitor in state.monitors.values() {
            assert!(monitor.workspaces.contains(monitor.active_workspace_id()));
            for id in monitor.workspaces.rows().iter().flatten() {
                assert!(state.workspaces.contains_key(id));
            }
        }
    }
}
