//! Tauri commands for Gen1Recomp Save Sync. The protocol itself lives in
//! `openhome_core::gen1recomp_sync`; this file only performs the HTTP requests
//! it describes and remembers which revision each open save was read at.

use crate::commands::CommandResult;
use crate::data_controller::ToDataController;
use openhome_core::gen1recomp_sync::{
    self as sync, HttpRequest, HttpResponse, OpenSyncSaves, RemoteSave, SyncAccount, SyncError,
};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Default)]
pub struct Gen1RecompSyncState(Mutex<OpenSyncSaves>);

const DEVICE_LABEL: &str = "OpenHome";

async fn send(request: HttpRequest) -> Result<HttpResponse, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(sync::TIMEOUT_SECONDS))
        .build()
        .map_err(|e| e.to_string())?;
    let method =
        reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|e| e.to_string())?;
    let mut builder = client.request(method, &request.url);
    for (name, value) in request.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = request.body {
        builder = builder.body(body);
    }
    let response = builder
        .send()
        .await
        .map_err(|e| format!("Could not reach the Gen1Recomp sync server: {e}"))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|e| e.to_string())?;
    Ok(HttpResponse { status, body })
}

fn linked_account(app_handle: &tauri::AppHandle) -> CommandResult<SyncAccount> {
    SyncAccount::load(&app_handle.controller())?
        .ok_or_else(|| SyncError::Unauthorized.to_string().into())
}

fn lock(state: &Gen1RecompSyncState) -> CommandResult<std::sync::MutexGuard<'_, OpenSyncSaves>> {
    state.0.lock().map_err(|e| e.to_string().into())
}

/// The label this device is linked under, or null when it is not linked.
#[tauri::command]
#[specta::specta]
pub fn gen1recomp_sync_status(app_handle: tauri::AppHandle) -> CommandResult<Option<String>> {
    Ok(SyncAccount::load(&app_handle.controller())?.map(|account| account.device_label))
}

/// Links OpenHome to the sync account using the two codes from Gen1Recomp's
/// SAVE SYNC screen.
#[tauri::command]
#[specta::specta]
pub async fn gen1recomp_sync_link(
    app_handle: tauri::AppHandle,
    code1: String,
    code2: String,
) -> CommandResult<()> {
    let request = sync::link_request(None, &code1, &code2, DEVICE_LABEL)?;
    let response = send(request).await?;
    let account = sync::parse_link(&response, None, DEVICE_LABEL)?;
    account.store(&app_handle.controller())?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn gen1recomp_sync_unlink(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Gen1RecompSyncState>,
) -> CommandResult<()> {
    let account = SyncAccount::load(&app_handle.controller())?;
    if let Some(account) = account {
        // Forgetting the credentials locally is what matters; the server-side
        // unlink is best effort, like upstream's own `SyncEngine:unlink`.
        let _ = send(sync::unlink_request(&account)).await;
    }
    SyncAccount::forget(&app_handle.controller())?;
    lock(&state)?.clear();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn gen1recomp_sync_list_saves(
    app_handle: tauri::AppHandle,
) -> CommandResult<Vec<RemoteSave>> {
    let account = linked_account(&app_handle)?;
    let response = send(sync::state_request(&account)).await?;
    Ok(sync::parse_state(&response)?)
}

/// Downloads a playthrough's save (its Lua source) and remembers the revision
/// it was read at, which the next write is checked against.
#[tauri::command]
#[specta::specta]
pub async fn gen1recomp_sync_load_save(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Gen1RecompSyncState>,
    version: String,
    playthrough_id: String,
) -> CommandResult<String> {
    let account = linked_account(&app_handle)?;
    let response = send(sync::get_save_request(&account, &version, &playthrough_id)).await?;
    let loaded = sync::parse_save(&response)?;
    let blob = loaded.blob.clone();
    lock(&state)?.remember(&version, &playthrough_id, loaded);
    Ok(blob)
}

/// Uploads a changed save over the revision it was read at. Refused (and
/// nothing written) if Gen1Recomp has uploaded that playthrough since.
#[tauri::command]
#[specta::specta]
pub async fn gen1recomp_sync_write_save(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Gen1RecompSyncState>,
    version: String,
    playthrough_id: String,
    blob: String,
) -> CommandResult<()> {
    let account = linked_account(&app_handle)?;
    let loaded = lock(&state)?
        .get(&version, &playthrough_id)
        .cloned()
        .ok_or_else(|| {
            format!("{version}/{playthrough_id} was not opened through Gen1Recomp Save Sync")
        })?;
    if loaded.blob == blob {
        return Ok(());
    }

    sync::backup_replaced_save(&app_handle.controller(), &version, &playthrough_id, &loaded)?;

    let request = sync::put_save_request(&account, &version, &playthrough_id, &loaded, &blob)?;
    let outcome = match send(request).await {
        Ok(response) if response.status < 500 => {
            sync::parse_put_save(&response).map_err(|e| e.to_string())
        }
        Ok(response) => Err(format!("the server answered {}", response.status)),
        Err(transport) => Err(transport),
    };

    let rev = match outcome {
        Ok(rev) => rev,
        Err(message) => {
            // A dropped connection or a server error can leave the upload's
            // outcome unknown. Reporting a write that actually landed as a
            // failure would make OpenHome roll back its side of the transfer,
            // so ask the server what it holds now before giving up.
            let check = send(sync::get_save_request(&account, &version, &playthrough_id)).await;
            match check.ok().and_then(|r| sync::parse_save(&r).ok()) {
                Some(current) if current.blob == blob => current.rev,
                _ => return Err(message.into()),
            }
        }
    };
    lock(&state)?.committed(&version, &playthrough_id, blob, rev);
    Ok(())
}
