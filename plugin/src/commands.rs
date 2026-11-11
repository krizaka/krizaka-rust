//! The commands the page calls through `@krizaka/app-desktop`. Each one goes through the kit's rules: the folder scope for
//! a path, the key and value rules for a secret, the menu checks for the menu bar.

use crate::{menu, watch, Kit, FOLDER_CHANGED};
use krizaka_rust::files::{self, Listing};
use krizaka_rust::menu::MenuSpec;
use krizaka_rust::settings;
use krizaka_rust::update::{self, Channel};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

type Res<T> = Result<T, String>;

fn paths_of(roots: &[PathBuf]) -> Vec<String> {
    roots.iter().map(|r| r.display().to_string()).collect()
}

fn save_scope(kit: &Kit, access: &krizaka_rust::access::Access) -> Res<()> {
    access
        .scope()
        .save(&kit.scope_file())
        .map_err(|e| e.to_string())
}

/// The folders the person granted.
#[command]
pub fn folders(kit: State<'_, Kit>) -> Res<Vec<String>> {
    Ok(paths_of(
        kit.access
            .lock()
            .map_err(|e| e.to_string())?
            .scope()
            .roots(),
    ))
}

/// The native folder picker; the chosen folder is granted if it may be (never the home, the system, `~/.ssh`…).
/// `None` when the person closed the picker.
#[command]
pub async fn grant_folder<R: Runtime>(
    app: AppHandle<R>,
    kit: State<'_, Kit>,
) -> Res<Option<String>> {
    let Some(picked) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let folder = picked.into_path().map_err(|e| e.to_string())?;
    let mut access = kit.access.lock().map_err(|e| e.to_string())?;
    let granted = access
        .scope_mut()
        .grant(&folder)
        .map_err(|e| e.to_string())?;
    save_scope(&kit, &access)?;
    Ok(Some(granted.display().to_string()))
}

/// Revokes a grant (and stops watching anything under it). Returns the folders still granted.
#[command]
pub fn revoke_folder(kit: State<'_, Kit>, path: String) -> Res<Vec<String>> {
    let root = PathBuf::from(&path);
    let roots = {
        let mut access = kit.access.lock().map_err(|e| e.to_string())?;
        access.scope_mut().revoke(&root);
        save_scope(&kit, &access)?;
        access.scope().roots().to_vec()
    };
    kit.watches
        .lock()
        .map_err(|e| e.to_string())?
        .retain(|watched, _| roots.iter().any(|r| watched.starts_with(r)));
    Ok(paths_of(&roots))
}

/// A folder's entries (folders first, hidden and partial files skipped, links never followed).
#[command]
pub fn list_folder(kit: State<'_, Kit>, path: String) -> Res<Listing> {
    files::list_dir(&kit.resolve(Path::new(&path))?)
}

/// Watches a granted folder: [`FOLDER_CHANGED`] is sent (debounced) when something in it changes. Watching twice is
/// watching once.
#[command]
pub fn watch_folder<R: Runtime>(
    app: AppHandle<R>,
    kit: State<'_, Kit>,
    path: String,
) -> Res<String> {
    let dir = kit.resolve(Path::new(&path))?;
    if !dir.is_dir() {
        return Err(format!("not a folder: {}", dir.display()));
    }
    let mut watches = kit.watches.lock().map_err(|e| e.to_string())?;
    if !watches.contains_key(&dir) {
        watches.insert(dir.clone(), watch::start(app, dir.clone(), FOLDER_CHANGED)?);
    }
    Ok(dir.display().to_string())
}

#[command]
pub fn unwatch_folder(kit: State<'_, Kit>, path: String) -> Res<()> {
    let mut watches = kit.watches.lock().map_err(|e| e.to_string())?;
    let target = Path::new(&path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&path));
    watches.remove(&target);
    Ok(())
}

/// A small secret from the Keychain (the session token): `null` when there is none.
#[command]
pub fn secret_get(kit: State<'_, Kit>, key: String) -> Res<Option<String>> {
    kit.secrets.get(&key)
}

#[command]
pub fn secret_set(kit: State<'_, Kit>, key: String, value: String) -> Res<()> {
    kit.secrets.set(&key, &value)
}

#[command]
pub fn secret_remove(kit: State<'_, Kit>, key: String) -> Res<()> {
    kit.secrets.remove(&key)
}

/// Replaces the menu bar with the one the page describes, in its language.
#[command]
pub fn set_menu<R: Runtime>(app: AppHandle<R>, kit: State<'_, Kit>, spec: MenuSpec) -> Res<()> {
    apply_menu(&app, &kit, &spec)
}

pub(crate) fn apply_menu<R: Runtime>(app: &AppHandle<R>, kit: &Kit, spec: &MenuSpec) -> Res<()> {
    spec.validate()?;
    let built = menu::build(app, spec).map_err(|e| e.to_string())?;
    app.set_menu(built).map_err(|e| e.to_string())?;
    *kit.menu_ids.lock().map_err(|e| e.to_string())? = spec
        .menus
        .iter()
        .flat_map(|m| m.items.iter().map(|i| i.id.clone()))
        .collect();
    Ok(())
}

/// The routes of the deep links that arrived since the last call (the app may have been opened by one before the page
/// listened). The page listens to `krizaka://open` first, then takes these.
#[command]
pub fn take_links(kit: State<'_, Kit>) -> Res<Vec<String>> {
    Ok(std::mem::take(
        &mut *kit.pending.lock().map_err(|e| e.to_string())?,
    ))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Prefs {
    #[serde(default)]
    channel: Channel,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    channel: Channel,
    /// The feed this channel reads, when the product has one.
    feed: Option<String>,
}

fn info(kit: &Kit, channel: Channel) -> Res<UpdateInfo> {
    let feed = match &kit.update_feed {
        Some(template) => Some(update::feed_url(template, channel)?.to_string()),
        None => None,
    };
    Ok(UpdateInfo { channel, feed })
}

/// The update channel on this Mac (`stable` unless the person chose `beta`).
#[command]
pub fn update_channel(kit: State<'_, Kit>) -> Res<UpdateInfo> {
    info(&kit, settings::load::<Prefs>(&kit.prefs_file()).channel)
}

#[command]
pub fn set_update_channel(kit: State<'_, Kit>, channel: Channel) -> Res<UpdateInfo> {
    let mut prefs = settings::load::<Prefs>(&kit.prefs_file());
    prefs.channel = channel;
    settings::save(&kit.prefs_file(), &prefs)?;
    info(&kit, channel)
}
