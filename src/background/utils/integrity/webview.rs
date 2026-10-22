use base64::Engine;
use tauri::webview_version;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::{app::get_app_handle, error::Result};

use super::IntegrityError;

// A cold WebView2 startup can exceed three seconds, especially in debug builds.
const WEBVIEW_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub fn validate_webview_runtime() -> std::result::Result<(), IntegrityError> {
    match webview_version() {
        Ok(version) => {
            let major: u32 = version
                .split('.')
                .next()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0);
            if major < 110 {
                Err(IntegrityError::WebviewRuntimeOutdated)
            } else {
                Ok(())
            }
        }
        Err(_) => Err(IntegrityError::WebviewRuntimeNotInstalled),
    }
}

pub fn show_not_installed_dialog(app: &tauri::AppHandle) -> Result<()> {
    let ok_pressed = app
        .dialog()
        .message(t!("runtime.not_found_description"))
        .title(t!("runtime.not_found"))
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCustom(
            t!("runtime.download").to_string(),
        ))
        .blocking_show();
    if ok_pressed {
        open_webview2_download()?;
    }
    Ok(())
}

pub fn show_outdated_dialog(app: &tauri::AppHandle) -> Result<()> {
    let ok_pressed = app
        .dialog()
        .message(t!("runtime.outdated_description", min_version = "110"))
        .title(t!("runtime.outdated"))
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCustom(
            t!("runtime.download").to_string(),
        ))
        .blocking_show();
    if ok_pressed {
        open_webview2_download()?;
    }
    Ok(())
}

fn open_webview2_download() -> Result<()> {
    let url = "https://developer.microsoft.com/en-us/microsoft-edge/webview2/?form=MA13LH#download";
    crate::windows_api::WindowsApi::execute(url.to_string(), None, None, false)
}

/// Try creating a webview window, tauri for some reason could panic stopping the setup hook and for some reason
/// the panic hook is not catching this so this implementation is a workaround for that.
pub async fn check_for_webview_optimal_state() -> bool {
    log::info!("Testing webview optimal state...");

    let (tx, rx) = tokio::sync::oneshot::channel();

    std::thread::spawn(move || {
        let _ = tx.send(create_integrity_webview());
    });

    wait_for_webview_check(rx, WEBVIEW_CHECK_TIMEOUT).await
}

fn create_integrity_webview() -> Result<()> {
    let label = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("@seelen/integrity");
    let window = tauri::WebviewWindowBuilder::new(
        get_app_handle(),
        &label,
        tauri::WebviewUrl::App("vanilla/integrity/index.html".into()),
    )
    .visible(false)
    .build()?;
    window.hwnd()?; // build could not fail so we check for the handle.
    window.destroy()?; // close the fake window
    Ok(())
}

async fn wait_for_webview_check(
    rx: tokio::sync::oneshot::Receiver<Result<()>>,
    timeout: std::time::Duration,
) -> bool {
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(Ok(()))) => {
            log::info!("Webview optimal state confirmed.");
            true
        }
        Ok(Ok(Err(err))) => {
            log::error!("Webview optimal state check failed: {err:?}");
            false
        }
        Ok(Err(err)) => {
            log::error!("Webview optimal state check worker stopped: {err}");
            false
        }
        Err(_) => {
            log::error!("Webview optimal state check timed out after {timeout:?}.");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_cold_start_longer_than_three_seconds() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(3100)).await;
                    tx.send(Ok(())).unwrap();
                });
                assert!(wait_for_webview_check(rx, WEBVIEW_CHECK_TIMEOUT).await);
            });
    }

    #[test]
    fn rejects_creation_failure_and_worker_exit() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                let (tx, rx) = tokio::sync::oneshot::channel();
                tx.send(Err("Webview creation failed".into())).unwrap();
                assert!(!wait_for_webview_check(rx, WEBVIEW_CHECK_TIMEOUT).await);

                let (tx, rx) = tokio::sync::oneshot::channel();
                drop(tx);
                assert!(!wait_for_webview_check(rx, WEBVIEW_CHECK_TIMEOUT).await);
            });
    }

    #[test]
    fn rejects_unresponsive_worker() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                let (_tx, rx) = tokio::sync::oneshot::channel();
                assert!(!wait_for_webview_check(rx, std::time::Duration::ZERO).await);
            });
    }
}
