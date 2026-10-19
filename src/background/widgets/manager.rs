use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicBool, Ordering},
};

use seelen_core::{
    resource::WidgetId,
    state::{WidgetLoader, WidgetStatus},
};

use crate::{
    error::{Result, ResultLogExt},
    modules::monitors::MonitorManager,
    resources::RESOURCES,
    state::application::FULL_STATE,
    utils::lock_free::SyncHashMap,
    widgets::{WidgetWebviewLabel, loader::WidgetDeployment},
};

pub static WIDGET_MANAGER: LazyLock<WidgetManager> = LazyLock::new(WidgetManager::create);
pub static GAME_MODE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct WidgetManager {
    /// group of widgets instances by widget resource id
    deployments: SyncHashMap<WidgetId, Arc<WidgetDeployment>>,
}

impl WidgetManager {
    fn create() -> Self {
        let sub_id = MonitorManager::subscribe(|_event| {
            WIDGET_MANAGER.reconcile().log_error();
        });
        MonitorManager::set_event_handler_priority(&sub_id, 1);
        Self {
            deployments: SyncHashMap::new(),
        }
    }

    pub fn is_ready(&self, label: &WidgetWebviewLabel) -> bool {
        self.deployment(&label.widget_id)
            .map(|deploy| deploy.pods.any(|(key, pod)| key == label && pod.is_ready()))
            .unwrap_or(false)
    }

    pub fn set_status(&self, label: &WidgetWebviewLabel, status: WidgetStatus) {
        if let Some(deploy) = self.deployment(&label.widget_id) {
            deploy.pods.get(label, |instance| {
                instance.set_status(status);
            });
        }
    }

    /// Release the registry lock before accessing pods or creating webviews.
    pub fn deployment(&self, id: &WidgetId) -> Option<Arc<WidgetDeployment>> {
        self.deployments.get(id, |deployment| deployment.clone())
    }

    pub fn deployments(&self) -> Vec<Arc<WidgetDeployment>> {
        self.deployments.values()
    }

    pub fn suspend_all(&self) {
        GAME_MODE_ACTIVE.store(true, Ordering::Release);
        for deploy in self.deployments() {
            deploy.pods.clear();
        }
    }

    pub fn resume_all(&self) -> Result<()> {
        GAME_MODE_ACTIVE.store(false, Ordering::Release);
        self.reconcile()
    }

    pub fn reconcile(&self) -> Result<()> {
        // remove deleted resources
        self.deployments
            .retain(|(key, _)| RESOURCES.widgets.contains_sync(key));

        let mut filtered = Vec::new();
        RESOURCES.widgets.iter_sync(|k, w| {
            if w.loader != WidgetLoader::Legacy {
                filtered.push((k.clone(), w.clone()));
            }
            true
        });

        let state = FULL_STATE.load();
        for (id, widget) in filtered {
            if !state.is_widget_enabled(&id) {
                self.deployments.remove(&id);
                continue;
            }

            if !self.deployments.contains_key(&id) {
                self.deployments
                    .upsert(id.clone(), Arc::new(WidgetDeployment::new(widget)));
            }
        }

        // lazy creation of webviews to reduce startup time
        std::thread::spawn(|| {
            fn reconcile(deployment: &WidgetDeployment) {
                deployment.reconcile();
                if !deployment.definition.lazy && !GAME_MODE_ACTIVE.load(Ordering::Acquire) {
                    deployment.start_all_webviews();
                }
            }

            // More visual widgets load first
            for priority in [
                WidgetId::known_wall(),
                WidgetId::known_toolbar(),
                WidgetId::known_weg(),
            ] {
                if let Some(deployment) = WIDGET_MANAGER.deployment(&priority) {
                    reconcile(&deployment);
                }
            }

            // All other widgets
            for deployment in WIDGET_MANAGER.deployments() {
                reconcile(&deployment);
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seelen_core::state::Widget;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn busy_widget_does_not_block_the_registry_during_monitor_changes() {
        let manager = WidgetManager {
            deployments: SyncHashMap::new(),
        };
        let widget = Widget {
            id: WidgetId::known_toolbar(),
            ..Default::default()
        };
        let id = widget.id.clone();
        manager.deployments.upsert(
            id.clone(),
            Arc::new(WidgetDeployment::new(Arc::new(widget))),
        );
        let (busy_tx, busy_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker_manager = &manager;
            let worker_id = id.clone();
            scope.spawn(move || {
                let deployment = worker_manager.deployment(&worker_id).unwrap();
                deployment.pods.with_lock(|_| {
                    busy_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                });
            });
            busy_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            // The monitor listener must be able to inspect/change the registry
            // even while webview work holds the deployment's pod lock.
            assert!(manager.deployment(&id).is_some());
            assert_eq!(manager.deployments().len(), 1);
            assert!(manager.deployments.remove(&id).is_some());
            release_tx.send(()).unwrap();
        });
    }
}
