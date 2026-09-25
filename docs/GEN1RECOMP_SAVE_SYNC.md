# Gen1Recomp Save Sync in OpenHome

OpenHome can act as the Pokémon storage app for [Gen1Recomp](https://github.com/bryanthaboi/gen1recomp),
filling the role [PokemonStorageSystem](https://github.com/logie-github/PokemonStorageSystem) (PSS) plays today.
It uses the Save Sync protocol that PSS already uses, with no changes to Gen1Recomp.

```
Gen1Recomp ⇄ Save Sync server ⇄ OpenHome (Gen1RecompSAV, PK1) ⇄ OpenHome storage / tracking / conversion
```

## 1. How PokemonStorageSystem's Save Sync works

This section comes from PSS's source (`sync/SyncApi.kt`, `sync/SaveRepository.kt`,
`transfer/TransferEngine.kt`) and Gen1Recomp's (`src/sync/SyncClient.lua`, `src/sync/SyncEngine.lua`,
`src/core/SaveSerializer.lua`).

- **No direct connection between the apps.** Gen1Recomp uploads whole saves to an HTTPS sync server
  (`https://sync.147.182.215.255.sslip.io`, or `POKEPORT_SYNC_URL`). PSS is just another device on
  that account. There are no sockets, ports, local discovery or Android IPC.
- **Pairing.** In Gen1Recomp, _Save Sync → Create Sync Account_ calls `POST /sync/create` and shows
  two 8-digit codes. The storage app sends them to `POST /sync/link` and gets back
  `{account, deviceToken, device}`. Every later call sends the headers `x-sync-account` and
  `x-sync-token`.
- **Unit of sync.** One whole playthrough save. Its key is `<version>/<meta.playthroughId>`, for
  example `red/quiet-forest-dawn`. The payload (`blob`) is the save file's Lua source, byte for byte
  what `SaveSerializer.encode` writes: `return { … }`.
- **Receiving Pokémon.** PSS downloads the save (`GET /sync/save`), removes the Pokémon from the
  parsed Lua tree, and uploads the whole save again (`PUT /sync/save`) with `baseRev` set to the
  revision it read.
- **Sending Pokémon back.** It works the same way: PSS inserts the Lua record into a box or the
  party and uploads the save.
- **Conflict handling.** The server refuses a stale `baseRev` with HTTP 409. PSS never sends
  `force`. It keeps a local backup of each blob it replaces, and a transfer journal settles
  "unknown" outcomes (a timeout after the body was sent) by comparing the save's hash afterwards.
- **Game side.** Gen1Recomp syncs 5 seconds after an in-game save, every 300 seconds, and when the
  app resumes (at least 60 seconds apart). When the server revision changed and the local save did
  not, it downloads and replaces the local save. When both changed, it asks the player, unless both
  sides show the same minute of play time, in which case its own copy wins.
- **Not used:** PSS also contains a "notes" mailbox (`/sync/notes`), but no screen calls it, and
  upstream Gen1Recomp has no code that reads notes. It is not part of the working protocol, so
  OpenHome does not implement it.

**What OpenHome has to reproduce:** link with two codes, list saves, download a save, parse and
write the Lua save format, map a Lua Pokémon to and from a Gen I Pokémon, and upload with `baseRev`.

## 2. Protocol and data structures

| Call                          | Request                                                      | Reply                                                                                                                         |
| ----------------------------- | ------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------- |
| `POST /sync/link`             | `{code1, code2, device}` (8 digits each, no auth)            | `{account, deviceToken, device}`                                                                                              |
| `GET /sync/state`             | auth headers                                                 | `{saves: {"<ver>/<id>": {rev, meta:{summary:{name,badges,timeText,dexCount}, savedAt, playTime, format}}}, devices, deleted}` |
| `GET /sync/save?id=&version=` | auth headers                                                 | `{blob, rev, meta, slot?}`                                                                                                    |
| `PUT /sync/save`              | `{version, blob, baseRev, meta, slot?}` (`force` never sent) | `{rev}`, or `409` if stale                                                                                                    |
| `POST /sync/unlink`           | `{device}`                                                   | —                                                                                                                             |

Status handling follows upstream `SyncClient:poll`: `401/403` means unlinked, `409` means conflict,
and any other `≥400` or an `error` field means failure. The blob limit is 2 MiB and the timeout is 25 s.

When OpenHome uploads, it sends back the `meta` it received without changing it. OpenHome did not
play the game, so claiming a new `savedAt` or play time would break Gen1Recomp's change detection.
PSS follows the same rule.

A save's Pokémon live in `boxes[1..12][1..20]` (older saves have a single `box` list) and `party[1..6]`.
A Pokémon is a Lua table. PK1 has no field that isn't in this table, and the table's extra fields
are kept verbatim (see §3).

## 3. Field mapping: Gen1Recomp Lua ↔ OpenHome `PK1`

The mapping mirrors Gen1Recomp's own `.sav` converter (`src/save_convert/GenSave.lua`
`decodeMon`/`encodeMon`), which maps this same table to and from the cartridge `party_struct` bytes
that PK1 reads.

| Lua field                                      | PK1 field                                 | Notes                                                                                                                                   |
| ---------------------------------------------- | ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `species` (pokered constant, e.g. `NIDORAN_M`) | `nationalDex`                             | `gen1Constants.ts` crosswalk. An unknown or modded species cannot be represented, so its slot is locked and the record is kept verbatim |
| `level`                                        | `level`                                   | PK1 recomputes level from `exp`, as it does for cartridge saves                                                                         |
| `exp`                                          | `exp`                                     |                                                                                                                                         |
| `hp`                                           | `currentHP`                               | Capped at max HP                                                                                                                        |
| `status` (`SLP/PSN/BRN/FRZ/PAR`)               | `statusCondition`                         | Same bits as GenSave.lua                                                                                                                |
| `typeBytes {t1,t2}`                            | `type1`, `type2`                          | Taken from species metadata when missing                                                                                                |
| `catchRate`                                    | `heldItemIndexGen1` (byte 0x07)           | Species base catch rate when missing                                                                                                    |
| `moves[i].id` / `.pp` / `.ppUps`               | `moves[i]` / `movePP[i]` / `movePPUps[i]` | Move constant ↔ move id crosswalk                                                                                                       |
| `otId`                                         | `trainerID`                               |                                                                                                                                         |
| `ot`                                           | `trainerName`                             | Player's name when missing, as GenSave.lua does                                                                                         |
| `nickname` (`nil` = not nicknamed)             | `nickname` (species name = not nicknamed) | Translated at the boundary (GenSave.lua #257)                                                                                           |
| `dvs {attack,defense,speed,special,hp}`        | `dvs {atk,def,spe,spc,hp}`                | HP DV is derived                                                                                                                        |
| `statExp {hp,attack,defense,speed,special}`    | `evsG12`                                  |                                                                                                                                         |
| `stats {…}`                                    | `getStats()`                              | Always written, so a box Pokémon is ready for the party                                                                                 |
| anything else (mod fields, `traded`, …)        | —                                         | Kept: a Pokémon that returns unchanged is written back from its original record, and a changed one is written over its own record       |
| —                                              | `gameOfOrigin` / `language`               | The save's Red/Blue/Yellow; English                                                                                                     |

OpenHome does not add a second Pokémon model. Once a Gen1Recomp Pokémon is a `PK1`, the normal
OHPKM tracking (`gen12` lookups), conversion and storage apply unchanged, including tracking when a
Pokémon moves between generations.

## 4. OpenHome files changed

- `openhome_core/src/lib.rs`: registers the module.
- `src-tauri/src/lib.rs`: registers the commands and manages `Gen1RecompSyncState`.
- `src/core/tauri/spectaCommands.ts`: bindings for the new commands and the `RemoteSave` type.
  Regenerate with the `export_typescript_bindings` test.
- `src/core/tauri/backend.ts`: `loadSaveFile`, `writeSaveFile` and `writeAllSaveFiles` send
  `gen1recomp-sync://` paths to Save Sync instead of the filesystem.
- `src/core/backend/backendInterface.ts`: link, unlink, status and list methods.
- `src/ui/state/appInfo.ts`: registers `Gen1RecompSAV` as a save type.
- `src/ui/state/saves/useSaves.ts`: keeps sync saves out of the recent-files list.
- `src/ui/saves/SavesModal.tsx`: adds a "Gen1Recomp Sync" tab.

## 5. New files

- `openhome_core/src/gen1recomp_sync.rs`: the protocol (request builders, reply parsers, credentials,
  revision tracking, local backups of replaced saves). It does no I/O, so it can be unit-tested.
- `src-tauri/src/gen1recomp_sync.rs`: Tauri commands. It runs the HTTP calls with reqwest,
  remembers the revision each open save was read at, and after an ambiguous upload failure
  re-reads the save before reporting an error.
- `src/core/save/gen1recomp/lua.ts`: reader and writer for the `SaveSerializer` grammar.
- `src/core/save/gen1recomp/gen1Constants.ts`: species and move constant crosswalks, from pret/pokered.
- `src/core/save/gen1recomp/gen1RecompMon.ts`: the Lua ↔ PK1 adapter.
- `src/core/save/gen1recomp/Gen1RecompSAV.ts`: the save class, PC boxes only, plus sync path helpers.
- `src/ui/saves/Gen1RecompSync.tsx`: link, list and open UI.
- Tests: `src/core/save/gen1recomp/__test__/Gen1RecompSAV.test.ts`, plus unit tests in `gen1recomp_sync.rs`.

## 6. Android requirements

The sync feature is platform-neutral: HTTPS through reqwest, credentials in OpenHome's storage
folder, and no desktop APIs. OpenHome's Tauri shell needed these changes to build for Android:

- The menu (`menu.rs`), the window-state plugin and the data-folder picker are desktop-only
  (`#[cfg(desktop)]`).
- `tauri.android.conf.json` gives Android the reverse-domain identifier it requires
  (`dev.andrewbenington.openhome`); the desktop identifier is unchanged.
- On mobile, `XDG_CONFIG_HOME`/`XDG_DATA_HOME` point at the app's own folders before startup,
  because the `dirs` crate otherwise resolves outside the Android sandbox.

Building an APK (arm64):

```sh
export ANDROID_HOME=<sdk> NDK_HOME=<sdk>/ndk/<version>
pnpm tauri android init          # src-tauri/gen is gitignored, so generate it once
pnpm tauri android build --apk --target aarch64
# then zipalign + apksigner the unsigned APK in src-tauri/gen/android/app/build/outputs/apk
```

Requirements specific to Save Sync:

- **Network:** the `INTERNET` permission, which Tauri's Android template already includes. The
  server is HTTPS, so no cleartext-traffic config is needed.
- **TLS:** reqwest 0.13's default verifier (`rustls-platform-verifier`) needs JNI setup on Android.
  The sync client instead trusts Mozilla's root store (`webpki-root-certs`) on Android. The
  existing plugin downloader still uses the default client, so plugin downloads may fail on
  Android.
- **Lifecycle and background sync:** none needed. OpenHome only contacts the server when the user
  opens or saves a playthrough, and `baseRev` rejects stale writes. Gen1Recomp's own sync (after
  saves, every 5 minutes, on resume) picks up OpenHome's writes.
- **Storage:** credentials go in `storage/gen1recomp_sync.json`, and backups of replaced saves go
  in `storage/gen1recomp_backups/` (10 kept per playthrough). Both are inside the app's private
  data directory.

## 7. Does Gen1Recomp need changes?

**No. Upstream Gen1Recomp can stay unchanged.** OpenHome uses only the endpoints and save format
that Gen1Recomp and PSS already use.

One behavior of the unmodified game to know about: if the player saves in Gen1Recomp after
OpenHome has written that playthrough, and before the game has downloaded OpenHome's version,
the two copies conflict. If both show the same minute of play time, Gen1Recomp keeps its own copy,
and a Pokémon moved into OpenHome can then exist in both places. PSS has the same limitation.
The safe workflow is: save in the game and let it sync, transfer in OpenHome, then let the game
sync (resume or wait) before playing on. Fixing this properly would need a change in Gen1Recomp's
conflict rule, which is out of scope here.

## 8. Implementation order (as built)

1. `openhome_core::gen1recomp_sync`: protocol types, requests and parsing (unit-tested).
2. `src-tauri` commands and state (HTTP, revision tracking, backups, ambiguous-failure check).
3. `lua.ts`: reads and writes the save format byte-compatibly with upstream.
4. `gen1Constants.ts` and `gen1RecompMon.ts`: the Lua ↔ PK1 adapter.
5. `Gen1RecompSAV`: PC boxes as PK1, verbatim write-back, locked slots for unknown records.
6. Backend routing of `gen1recomp-sync://` paths, save-type registration, and the UI tab.
7. Android shell bring-up (§6).

## Scope limits

- Only Generation I playthroughs (Red/Blue/Yellow). Gold/Silver/Crystal saves are listed but
  cannot be opened; PK2 support would need its own adapter.
- Only PC boxes are shown, not the party, the same as `G1SAV`.
- A local Gen1Recomp `save.lua` file can also be opened directly with _Open File_.
