use std::{path::PathBuf, time::Duration};

use slu_ipc::{IPC, ServiceIpc, messages::SvcAction};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use windows::Win32::{
    System::TaskScheduler::{IExecAction2, ITaskService, TaskScheduler},
    UI::Shell::FOLDERID_LocalAppData,
};
use windows_core::Interface;

use crate::{
    app::get_app_handle,
    error::Result,
    get_tokio_handle,
    utils::was_installed_using_msix,
    windows_api::{Com, WindowsApi},
};

pub struct ServicePipe;

impl ServicePipe {
    /// will ignore any response
    pub fn request(message: SvcAction) -> Result<()> {
        get_tokio_handle().spawn(async move {
            if let Err(err) = ServiceIpc::send(message.clone()).await {
                log::error!("Error sending message to service {err}. Message: {message:?}");
            }
        });
        Ok(())
    }

    pub fn request_sync(message: SvcAction) -> Result<()> {
        ServiceIpc::send_blocking(message.clone())?.ok()?;
        Ok(())
    }

    pub fn is_running() -> bool {
        ServiceIpc::can_stablish_connection()
    }

    pub fn service_path() -> Result<PathBuf> {
        let service_path = if was_installed_using_msix() {
            WindowsApi::known_folder(FOLDERID_LocalAppData)?
                .join("Microsoft\\WindowsApps\\slu-service.exe")
        } else {
            std::env::current_exe()?.with_file_name("slu-service.exe")
        };
        Ok(service_path)
    }

    fn try_start_from_task_scheduler() -> Result<()> {
        Com::run_with_context(|| unsafe {
            let task_service: ITaskService = Com::create_instance(&TaskScheduler)?;
            task_service.Connect(
                &Default::default(),
                &Default::default(),
                &Default::default(),
                &Default::default(),
            )?;
            let folder = task_service.GetFolder(&"\\Seelen".into())?;
            let task = folder.GetTask(&"Seelen UI Service".into())?;

            let actions = task.Definition()?.Actions()?;
            // ask to microsoft what that hell this start counting from 1 instead 0
            let action: IExecAction2 = actions.get_Item(1)?.cast()?;
            let mut action_path = windows_core::BSTR::new();
            action.Path(&mut action_path)?;

            let service_path = Self::service_path()?.to_string_lossy().to_lowercase();
            match action_path.to_string().to_lowercase() == service_path {
                true => {
                    task.Run(&Default::default())?;
                    Ok(())
                }
                false => {
                    Err("Service task is not pointing to the correct service executable".into())
                }
            }
        })
    }

    // the service should be running since installer will start it or startup task scheduler
    // so if the service is not running, we need to start it (common on msix setup)
    pub async fn start_service() -> Result<()> {
        if let Err(err) = Self::try_start_from_task_scheduler() {
            log::debug!("Can not start service via task scheduler: {err}");
            let service_path = Self::service_path()?;
            let result =
                WindowsApi::execute(service_path.to_string_lossy().to_string(), None, None, true);

            if let Err(err) = result {
                log::error!("First attempt to start service failed: {err}");
                let try_again = get_app_handle()
                    .dialog()
                    .message(t!("service.not_running_description"))
                    .title(t!("service.not_running"))
                    .kind(MessageDialogKind::Info)
                    .buttons(MessageDialogButtons::OkCustom(
                        t!("service.not_running_ok").to_string(),
                    ))
                    .blocking_show();

                if !try_again {
                    return Err(err);
                }
                WindowsApi::execute(service_path.to_string_lossy().to_string(), None, None, true)?;
            }
        }

        // Task.Run only queues a launch. Both launch paths must wait for IPC before
        // app initialization sends its one-time shortcut registration.
        if !wait_for_service_ready(
            Self::is_running,
            Duration::from_secs(30),
            Duration::from_millis(100),
        )
        .await
        {
            get_app_handle()
                .dialog()
                .message(t!("service.not_running_description"))
                .title(t!("service.not_running"))
                .kind(MessageDialogKind::Error)
                .buttons(MessageDialogButtons::Ok)
                .blocking_show();
            return Err("Service not running".into());
        }

        Ok(())
    }
}

async fn wait_for_service_ready(
    mut is_running: impl FnMut() -> bool,
    timeout: Duration,
    interval: Duration,
) -> bool {
    tokio::time::timeout(timeout, async {
        while !is_running() {
            log::debug!("Waiting for service IPC...");
            tokio::time::sleep(interval).await;
        }
    })
    .await
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn service_readiness_waits_for_delayed_startup() {
        let mut probes = 0;
        assert!(
            wait_for_service_ready(
                || {
                    probes += 1;
                    probes >= 3
                },
                Duration::from_secs(1),
                Duration::from_millis(1),
            )
            .await
        );
        assert_eq!(probes, 3);
    }

    #[tokio::test]
    async fn service_readiness_accepts_an_already_running_service() {
        assert!(
            wait_for_service_ready(|| true, Duration::from_secs(1), Duration::from_secs(1)).await
        );
    }

    #[tokio::test]
    async fn service_readiness_times_out_when_ipc_never_appears() {
        assert!(
            !wait_for_service_ready(|| false, Duration::from_millis(5), Duration::from_millis(1))
                .await
        );
    }
}
