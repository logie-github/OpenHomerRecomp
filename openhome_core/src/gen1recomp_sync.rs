//! Client side of Gen1Recomp's Save Sync protocol.
//!
//! This is the same protocol Gen1Recomp's own `src/sync/SyncClient.lua` speaks
//! and that PokemonStorageSystem reuses: OpenHome links to the player's sync
//! account as one more device (using the two 8-digit codes shown on the
//! game's SAVE SYNC screen), lists the account's playthroughs, downloads a
//! save's Lua source, and uploads it again after Pokémon have been moved.
//!
//! Every write carries the `baseRev` the save was read at, so the server
//! refuses (409) a write over a save the game has changed since. `force` is
//! never sent.
//!
//! Nothing in here performs I/O. Requests are built as [`HttpRequest`]s and
//! responses are parsed from [`HttpResponse`]s, so the Tauri layer owns the
//! network and this module stays testable.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::data_controller::{DataController, DataDir};
use crate::error::{Error, Result};

/// Upstream `SyncClient.DEFAULT_URL`.
pub const DEFAULT_URL: &str = "https://sync.147.182.215.255.sslip.io";
/// Upstream `SyncClient.MAX_BLOB`.
pub const MAX_BLOB_BYTES: usize = 2 * 1024 * 1024;
/// Upstream `SyncClient.TIMEOUT`, in seconds.
pub const TIMEOUT_SECONDS: u64 = 25;

const ACCOUNT_FILENAME: &str = "gen1recomp_sync.json";
const BACKUP_DIR: &str = "gen1recomp_backups";
const BACKUPS_PER_SAVE: usize = 10;

/// Game versions whose Pokémon OpenHome reads as PK1.
pub const GEN1_VERSIONS: [&str; 3] = ["red", "blue", "yellow"];

/// Upstream `SyncClient.normalizeCode`: exactly eight digits, separators ignored.
pub fn normalize_code(code: &str) -> Option<String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    (digits.len() == 8).then_some(digits)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// This device's credentials on the player's sync account.
///
/// Exactly as powerful as the two sync codes: they read and write every save
/// on the account, so they are kept in OpenHome's storage folder and nowhere
/// else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncAccount {
    pub account: String,
    pub device_token: String,
    pub device_id: Option<String>,
    pub device_label: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl SyncAccount {
    pub fn load(controller: &impl DataController) -> Result<Option<SyncAccount>> {
        controller
            .read_file_json_if_exists(DataDir::Storage, ACCOUNT_FILENAME)
            .transpose()
    }

    pub fn store(&self, controller: &impl DataController) -> Result<()> {
        controller.write_file_json(DataDir::Storage, ACCOUNT_FILENAME, self)
    }

    pub fn forget(controller: &impl DataController) -> Result<()> {
        let path = controller.absolute_path(DataDir::Storage, ACCOUNT_FILENAME)?;
        if path.exists() {
            controller.delete_file(DataDir::Storage, ACCOUNT_FILENAME)?;
        }
        Ok(())
    }

    fn base_url(&self) -> &str {
        self.base_url.as_deref().unwrap_or(DEFAULT_URL)
    }
}

/// What the server lists for one playthrough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "desktop", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RemoteSave {
    /// `<version>/<playthroughId>`, the server's key.
    pub key: String,
    pub version: String,
    pub playthrough_id: String,
    pub rev: f64,
    pub trainer_name: Option<String>,
    pub badges: Option<u32>,
    pub time_text: Option<String>,
    /// Whether OpenHome can open this playthrough (Generation I only).
    pub supported: bool,
}

/// A downloaded save: its Lua source and the revision it was read at.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveBlob {
    pub blob: String,
    pub rev: i64,
    pub meta: Option<Value>,
    pub slot: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// The device is not linked, or its token was revoked from another device.
    Unauthorized,
    /// The server moved on since the save was read (HTTP 409).
    Conflict,
    /// Anything else, with the server's explanation where it gave one.
    Failed { status: u16, message: String },
    /// The reply was not what the protocol promises.
    Malformed(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Unauthorized => write!(
                f,
                "OpenHome is no longer linked to this Gen1Recomp sync account. Link it again with new codes."
            ),
            SyncError::Conflict => write!(
                f,
                "Gen1Recomp saved over this playthrough after OpenHome opened it. Nothing was written; close the save in OpenHome, reopen it, and try again."
            ),
            SyncError::Failed { status, message } => {
                write!(f, "Gen1Recomp Save Sync failed ({status}): {message}")
            }
            SyncError::Malformed(why) => {
                write!(f, "Gen1Recomp Save Sync sent an unexpected reply: {why}")
            }
        }
    }
}

impl std::error::Error for SyncError {}

impl From<SyncError> for Error {
    fn from(value: SyncError) -> Self {
        Error::other_with_source("Gen1Recomp Save Sync", value)
    }
}

pub type SyncResult<T> = core::result::Result<T, SyncError>;

fn url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// Upstream `SyncClient` query escaping: everything but `[A-Za-z0-9-._~]`.
fn escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn json_request(method: &'static str, url: String, body: Option<Value>) -> HttpRequest {
    let mut headers = vec![("Accept", "application/json".to_owned())];
    if body.is_some() {
        headers.push(("Content-Type", "application/json".to_owned()));
    }
    HttpRequest {
        method,
        url,
        headers,
        body: body.map(|b| b.to_string()),
    }
}

fn authenticated(mut request: HttpRequest, account: &SyncAccount) -> HttpRequest {
    request
        .headers
        .push(("x-sync-account", account.account.clone()));
    request
        .headers
        .push(("x-sync-token", account.device_token.clone()));
    request
}

/// `POST /sync/link`: joins the account the two codes belong to.
pub fn link_request(
    base_url: Option<&str>,
    code1: &str,
    code2: &str,
    device_label: &str,
) -> SyncResult<HttpRequest> {
    let (Some(a), Some(b)) = (normalize_code(code1), normalize_code(code2)) else {
        return Err(SyncError::Malformed("both codes are 8 digits".to_owned()));
    };
    let body = json!({ "code1": a, "code2": b, "device": device_label });
    Ok(json_request(
        "POST",
        url(base_url.unwrap_or(DEFAULT_URL), "/sync/link"),
        Some(body),
    ))
}

pub fn parse_link(
    response: &HttpResponse,
    base_url: Option<&str>,
    device_label: &str,
) -> SyncResult<SyncAccount> {
    let json = check(response)?;
    let account = json.get("account").and_then(Value::as_str);
    let token = json.get("deviceToken").and_then(Value::as_str);
    match (account, token) {
        (Some(account), Some(token)) if !account.is_empty() && !token.is_empty() => {
            Ok(SyncAccount {
                account: account.to_owned(),
                device_token: token.to_owned(),
                device_id: non_empty_str(&json, "device"),
                device_label: device_label.to_owned(),
                base_url: base_url.map(str::to_owned),
            })
        }
        _ => Err(SyncError::Malformed(
            "no account in the link reply".to_owned(),
        )),
    }
}

/// `POST /sync/unlink` for this device.
pub fn unlink_request(account: &SyncAccount) -> HttpRequest {
    let mut body = Map::new();
    if let Some(device) = &account.device_id {
        body.insert("device".to_owned(), Value::String(device.clone()));
    }
    authenticated(
        json_request(
            "POST",
            url(account.base_url(), "/sync/unlink"),
            Some(Value::Object(body)),
        ),
        account,
    )
}

/// `GET /sync/state`: every playthrough on the account.
pub fn state_request(account: &SyncAccount) -> HttpRequest {
    authenticated(
        json_request("GET", url(account.base_url(), "/sync/state"), None),
        account,
    )
}

pub fn parse_state(response: &HttpResponse) -> SyncResult<Vec<RemoteSave>> {
    let json = check(response)?;
    let mut saves = Vec::new();
    if let Some(rows) = json.get("saves").and_then(Value::as_object) {
        for (key, row) in rows {
            let Some((version, playthrough_id)) = key.split_once('/') else {
                continue;
            };
            if playthrough_id.is_empty() {
                continue;
            }
            let meta = meta_of(row);
            let summary = meta.get("summary");
            saves.push(RemoteSave {
                key: key.clone(),
                version: version.to_owned(),
                playthrough_id: playthrough_id.to_owned(),
                rev: row.get("rev").and_then(Value::as_f64).unwrap_or(0.0),
                trainer_name: summary.and_then(|s| non_empty_str(s, "name")),
                badges: summary
                    .and_then(|s| s.get("badges"))
                    .and_then(Value::as_u64)
                    .map(|b| b as u32),
                time_text: summary.and_then(|s| non_empty_str(s, "timeText")),
                supported: GEN1_VERSIONS.contains(&version.to_ascii_lowercase().as_str()),
            });
        }
    }
    saves.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(saves)
}

/// `GET /sync/save?id=..&version=..` (upstream sorts the query names).
pub fn get_save_request(account: &SyncAccount, version: &str, playthrough_id: &str) -> HttpRequest {
    let path = format!(
        "/sync/save?id={}&version={}",
        escape(playthrough_id),
        escape(version)
    );
    authenticated(
        json_request("GET", url(account.base_url(), &path), None),
        account,
    )
}

pub fn parse_save(response: &HttpResponse) -> SyncResult<SaveBlob> {
    let json = check(response)?;
    let blob = json
        .get("blob")
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| SyncError::Malformed("the server sent no save data".to_owned()))?;
    let meta = json
        .get("meta")
        .or_else(|| json.get("remoteMeta"))
        .filter(|m| m.is_object())
        .cloned();
    Ok(SaveBlob {
        blob: blob.to_owned(),
        rev: json.get("rev").and_then(Value::as_i64).unwrap_or(0),
        slot: slot_of(&json),
        meta,
    })
}

/// `PUT /sync/save`: uploads [`blob`] over the revision it was read at.
///
/// `meta` is the metadata the server returned with the save, sent back
/// unchanged apart from `playthroughId`. OpenHome only moves Pokémon between
/// boxes; it did not play the game, so it must not claim a new `savedAt` or
/// play time, which Gen1Recomp's own change detection relies on.
pub fn put_save_request(
    account: &SyncAccount,
    version: &str,
    playthrough_id: &str,
    loaded: &SaveBlob,
    blob: &str,
) -> SyncResult<HttpRequest> {
    if blob.len() > MAX_BLOB_BYTES {
        return Err(SyncError::Failed {
            status: 0,
            message: "this save is too large to sync".to_owned(),
        });
    }
    let mut meta = match &loaded.meta {
        Some(Value::Object(meta)) => meta.clone(),
        _ => Map::new(),
    };
    meta.insert(
        "playthroughId".to_owned(),
        Value::String(playthrough_id.to_owned()),
    );
    let mut body = Map::new();
    body.insert("version".to_owned(), Value::String(version.to_owned()));
    body.insert("blob".to_owned(), Value::String(blob.to_owned()));
    body.insert("baseRev".to_owned(), Value::from(loaded.rev));
    body.insert("meta".to_owned(), Value::Object(meta));
    if let Some(slot) = &loaded.slot {
        body.insert("slot".to_owned(), Value::String(slot.clone()));
    }
    Ok(authenticated(
        json_request(
            "PUT",
            url(account.base_url(), "/sync/save"),
            Some(Value::Object(body)),
        ),
        account,
    ))
}

/// The new revision the server gave the uploaded save.
pub fn parse_put_save(response: &HttpResponse) -> SyncResult<i64> {
    let json = check(response)?;
    Ok(json.get("rev").and_then(Value::as_i64).unwrap_or(0))
}

/// Status and error handling shared by every endpoint, per upstream
/// `SyncClient:poll`: an `error` field or a 4xx/5xx is a failure.
fn check(response: &HttpResponse) -> SyncResult<Value> {
    let json: Option<Value> = serde_json::from_str(&response.body).ok();
    match response.status {
        401 | 403 => return Err(SyncError::Unauthorized),
        409 => return Err(SyncError::Conflict),
        _ => {}
    }
    let error = json
        .as_ref()
        .and_then(|j| j.get("error"))
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
        .map(str::to_owned);
    if response.status >= 400 || error.is_some() {
        return Err(SyncError::Failed {
            status: response.status,
            message: error.unwrap_or_else(|| format!("the server answered {}", response.status)),
        });
    }
    match json {
        Some(json @ Value::Object(_)) => Ok(json),
        _ => Err(SyncError::Malformed("unreadable reply".to_owned())),
    }
}

/// Upstream `SyncEngine.metaOf`: inline, under `meta`, or under `remoteMeta`.
fn meta_of(row: &Value) -> &Value {
    row.get("meta")
        .filter(|m| m.is_object())
        .or_else(|| row.get("remoteMeta").filter(|m| m.is_object()))
        .unwrap_or(row)
}

fn slot_of(row: &Value) -> Option<String> {
    [Some(row), row.get("meta"), row.get("remoteMeta")]
        .into_iter()
        .flatten()
        .find_map(|place| non_empty_str(place, "slot"))
}

fn non_empty_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Revisions of the saves OpenHome currently has open, keyed by server key.
///
/// A save is written against the revision it was read at; this is where that
/// revision (and the metadata to send back beside it) waits between the two.
#[derive(Debug, Default)]
pub struct OpenSyncSaves(HashMap<String, SaveBlob>);

impl OpenSyncSaves {
    pub fn key(version: &str, playthrough_id: &str) -> String {
        format!("{version}/{playthrough_id}")
    }

    pub fn remember(&mut self, version: &str, playthrough_id: &str, loaded: SaveBlob) {
        self.0.insert(Self::key(version, playthrough_id), loaded);
    }

    pub fn get(&self, version: &str, playthrough_id: &str) -> Option<&SaveBlob> {
        self.0.get(&Self::key(version, playthrough_id))
    }

    /// Records a successful upload: the uploaded bytes at their new revision.
    pub fn committed(&mut self, version: &str, playthrough_id: &str, blob: String, rev: i64) {
        if let Some(entry) = self.0.get_mut(&Self::key(version, playthrough_id)) {
            entry.blob = blob;
            entry.rev = rev;
        }
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Keeps a local copy of the save bytes an upload is about to replace, so a
/// playthrough can always be restored from this device (the equivalent of
/// the `.bak` Gen1Recomp keeps beside a local save).
pub fn backup_replaced_save(
    controller: &impl DataController,
    version: &str,
    playthrough_id: &str,
    loaded: &SaveBlob,
) -> Result<()> {
    let folder_name: String = format!("{version}~{playthrough_id}")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '~') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let folder = controller
        .absolute_path(DataDir::Storage, BACKUP_DIR)?
        .join(folder_name);
    std::fs::create_dir_all(&folder).map_err(|e| Error::file_write(&folder, e))?;

    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let file = folder.join(format!("{}-{millis}.lua", loaded.rev));
    std::fs::write(&file, &loaded.blob).map_err(|e| Error::file_write(&file, e))?;

    let mut existing: Vec<_> = std::fs::read_dir(&folder)
        .map_err(|e| Error::file_access(&folder, e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "lua"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    existing.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, stale) in existing.into_iter().skip(BACKUPS_PER_SAVE) {
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> SyncAccount {
        SyncAccount {
            account: "acct".to_owned(),
            device_token: "tok".to_owned(),
            device_id: Some("dev1".to_owned()),
            device_label: "OpenHome".to_owned(),
            base_url: Some("https://example.test/".to_owned()),
        }
    }

    fn ok(body: Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_string(),
        }
    }

    #[test]
    fn codes_are_eight_digits() {
        assert_eq!(normalize_code("1234-5678").as_deref(), Some("12345678"));
        assert_eq!(normalize_code("1234 567"), None);
    }

    #[test]
    fn link_round_trip() {
        let request = link_request(None, "1234-5678", "8765 4321", "OpenHome").unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, format!("{DEFAULT_URL}/sync/link"));
        let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["code1"], "12345678");
        assert_eq!(body["code2"], "87654321");
        assert!(!request.headers.iter().any(|(k, _)| *k == "x-sync-token"));

        let linked = parse_link(
            &ok(json!({ "account": "a1", "deviceToken": "t1", "device": "d1" })),
            None,
            "OpenHome",
        )
        .unwrap();
        assert_eq!(linked.account, "a1");
        assert_eq!(linked.device_token, "t1");
        assert_eq!(linked.device_id.as_deref(), Some("d1"));
    }

    #[test]
    fn state_lists_saves_and_marks_gen1() {
        let response = ok(json!({
            "saves": {
                "red/quiet-forest-dawn": {
                    "rev": 7,
                    "meta": { "summary": { "name": "ASH", "badges": 3, "timeText": "2:01" } }
                },
                "gold/other": { "rev": 2, "summary": { "name": "GOLD" } }
            },
            "devices": []
        }));
        let saves = parse_state(&response).unwrap();
        assert_eq!(saves.len(), 2);
        assert_eq!(saves[1].key, "red/quiet-forest-dawn");
        assert_eq!(saves[1].trainer_name.as_deref(), Some("ASH"));
        assert_eq!(saves[1].badges, Some(3));
        assert_eq!(saves[1].rev, 7.0);
        assert!(saves[1].supported);
        assert_eq!(saves[0].trainer_name.as_deref(), Some("GOLD"));
        assert!(!saves[0].supported);
    }

    #[test]
    fn get_and_put_carry_auth_and_base_rev() {
        let get = get_save_request(&account(), "red", "a b");
        assert_eq!(
            get.url,
            "https://example.test/sync/save?id=a%20b&version=red"
        );
        assert!(get.headers.contains(&("x-sync-account", "acct".to_owned())));
        assert!(get.headers.contains(&("x-sync-token", "tok".to_owned())));

        let loaded = parse_save(&ok(json!({
            "blob": "return {}\n",
            "rev": 12,
            "meta": { "savedAt": 1700000000, "playTime": 60.5, "slot": "2" }
        })))
        .unwrap();
        assert_eq!(loaded.rev, 12);
        assert_eq!(loaded.slot.as_deref(), Some("2"));

        let put = put_save_request(&account(), "red", "a b", &loaded, "return {x = 1,}\n").unwrap();
        assert_eq!(put.method, "PUT");
        let body: Value = serde_json::from_str(put.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["baseRev"], 12);
        assert_eq!(body["version"], "red");
        assert_eq!(body["slot"], "2");
        assert_eq!(body["meta"]["savedAt"], 1700000000);
        assert_eq!(body["meta"]["playthroughId"], "a b");
        assert!(body.get("force").is_none());
    }

    #[test]
    fn statuses_map_to_errors() {
        let conflict = HttpResponse {
            status: 409,
            body: "{\"rev\":13}".to_owned(),
        };
        assert_eq!(parse_put_save(&conflict), Err(SyncError::Conflict));
        let unauthorized = HttpResponse {
            status: 401,
            body: String::new(),
        };
        assert_eq!(parse_state(&unauthorized), Err(SyncError::Unauthorized));
        let error = HttpResponse {
            status: 200,
            body: "{\"error\":\"nope\"}".to_owned(),
        };
        assert_eq!(
            parse_state(&error),
            Err(SyncError::Failed {
                status: 200,
                message: "nope".to_owned()
            })
        );
    }

    #[test]
    fn open_saves_track_revisions() {
        let mut open = OpenSyncSaves::default();
        open.remember(
            "red",
            "p",
            SaveBlob {
                blob: "a".to_owned(),
                rev: 1,
                meta: None,
                slot: None,
            },
        );
        open.committed("red", "p", "b".to_owned(), 2);
        let entry = open.get("red", "p").unwrap();
        assert_eq!(entry.rev, 2);
        assert_eq!(entry.blob, "b");
    }
}
