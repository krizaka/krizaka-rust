//! The Tauri side of `krizaka-rust`: one plugin a Krizaka Mac app adds, so its own `lib.rs` only holds what is the
//! product's (its commands, each resolving its paths through [`Kit::resolve`]).
//!
//! ```ignore
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_krizaka::dialog::init())
//!     .plugin(
//!         tauri_plugin_krizaka::Builder::new("verikora")
//!             .link_hosts(["www.verikora.com", "dev.verikora.com"])
//!             .link_prefixes(["/app", "/verify"])
//!             .build(),
//!     )
//!     .invoke_handler(tauri::generate_handler![read_document])
//!     .run(tauri::generate_context!())
//! ```
//!
//! What it brings, and what the page reaches through `@krizaka/app-desktop` (capability `krizaka:default`):
//! - folders the person grants with the native picker, listed, revoked, watched; files dropped on the window
//!   (event [`DROPPED`]); every path through the kit's one gate ([`krizaka_rust::access::Access`]);
//! - small secrets in the macOS Keychain (the session token), the `KeyValueStorage` of `@krizaka/app-client`;
//! - the native menu bar the page describes in its own words ([`krizaka_rust::menu::MenuSpec`]), clicks sent back
//!   as [`MENU`];
//! - deep links (`<scheme>://…` and the product's web links) turned into a route ([`OPEN`], and [`take_links`] for the
//!   ones that arrived before the page listened);
//! - the navigation guard on every window: the app's own places stay, web links open in the browser, the rest is refused;
//! - the update channel (`stable` / `beta`) and the feed it reads.

mod commands;
mod error;
mod menu;
mod watch;

pub use krizaka_rust as kit;
/// The folder picker the kit's commands open. The app registers it next to the kit —
/// `.plugin(tauri_plugin_krizaka::dialog::init())` — and needs no dependency of its own: a plugin registered from inside
/// another plugin's setup deadlocks Tauri's plugin store (found running the first app).
pub use tauri_plugin_dialog as dialog;

use krizaka_rust::access::Access;
use krizaka_rust::deep_link::{self, LinkRules};
use krizaka_rust::nav_guard::{self, Navigation};
use krizaka_rust::scope::{ScopeStore, ScopedPath};
use krizaka_rust::secrets::{Checked, SecretStore};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, Runtime, State, WindowEvent};

pub use commands::take_links;
pub use error::{CommandError, CODES};

/// Paths the system reports as dropped on a window, already allowed by the gate: `string[]`.
pub const DROPPED: &str = "krizaka://dropped";
/// Files are being dragged over a window (`true`), or no longer are (`false`): the page shows where to drop.
pub const DRAGGING: &str = "krizaka://dragging";
/// A deep link's route (`/app/rapports`): `string`.
pub const OPEN: &str = "krizaka://open";
/// The id of a menu item the person chose: `string`.
pub const MENU: &str = "krizaka://menu";
/// A watched folder whose content changed (debounced): its path, `string`.
pub const FOLDER_CHANGED: &str = "krizaka://folder-changed";

const SCOPE_FILE: &str = "scope.json";
const PREFS_FILE: &str = "krizaka.json";

/// The kit's state in the app. A product's own command reads it with [`KitExt::kit`].
pub struct Kit {
    access: Mutex<Access>,
    secrets: Box<dyn SecretStore>,
    links: LinkRules,
    pending: Mutex<Vec<String>>,
    hosted: Vec<String>,
    /// The window that shows the hosted app: a link navigates it (its page cannot listen, it has no IPC).
    hosted_window: Option<String>,
    update_feed: Option<String>,
    data_dir: PathBuf,
    menu_ids: Mutex<HashSet<String>>,
    watches: Mutex<HashMap<PathBuf, watch::Watch>>,
}

impl Kit {
    /// The one gate for a product's own command: a file of a granted folder, or a file dropped this session. Read it with
    /// [`ScopedPath::open`], which confirms the opened file is still the one resolved.
    pub fn resolve(&self, path: &Path) -> Result<ScopedPath, CommandError> {
        Ok(error::lock(&self.access)?.resolve(path)?)
    }

    fn scope_file(&self) -> PathBuf {
        self.data_dir.join(SCOPE_FILE)
    }

    fn prefs_file(&self) -> PathBuf {
        self.data_dir.join(PREFS_FILE)
    }
}

/// `app.kit()` (or `window.kit()`) in a product's command.
pub trait KitExt<R: Runtime> {
    fn kit(&self) -> State<'_, Kit>;
}

impl<R: Runtime, T: Manager<R>> KitExt<R> for T {
    fn kit(&self) -> State<'_, Kit> {
        self.state::<Kit>()
    }
}

/// The plugin, configured with the product's own addresses.
pub struct Builder {
    scheme: String,
    hosts: Vec<String>,
    prefixes: Vec<String>,
    hosted: Vec<String>,
    hosted_window: Option<String>,
    update_feed: Option<String>,
}

impl Builder {
    /// `scheme` is the product's deep-link scheme, lowercase (`verikora`), the one its Info.plist declares.
    pub fn new(scheme: &str) -> Self {
        Builder {
            scheme: scheme.to_ascii_lowercase(),
            hosts: vec![],
            prefixes: vec![],
            hosted: vec![],
            hosted_window: None,
            update_feed: None,
        }
    }

    /// The product's web hosts whose links open the app (`www.verikora.com`).
    pub fn link_hosts<I: IntoIterator<Item = S>, S: Into<String>>(mut self, hosts: I) -> Self {
        self.hosts = hosts.into_iter().map(Into::into).collect();
        self
    }

    /// The routes a link may open (`/app`); none means every route.
    pub fn link_prefixes<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        prefixes: I,
    ) -> Self {
        self.prefixes = prefixes.into_iter().map(Into::into).collect();
        self
    }

    /// A hosted web app a window of this app may show (`https://www.folioviv.com`): HTTPS, reduced to its origin.
    pub fn hosted_origin(mut self, origin: &str) -> Self {
        self.hosted.push(origin.to_string());
        self
    }

    /// The window (its label) that shows the first hosted origin: a deep link navigates it to the link's route on that
    /// origin. A hosted page gets no capability, so it is told nothing; the window just goes there.
    pub fn hosted_window(mut self, label: &str) -> Self {
        self.hosted_window = Some(label.to_string());
        self
    }

    /// The update feed, with `{channel}` (see [`krizaka_rust::update::feed_url`]). The app's `plugins.updater` must
    /// then hold a `pubkey`, or the plugin refuses to start (see [`krizaka_rust::update`]).
    pub fn update_feed(mut self, template: &str) -> Self {
        self.update_feed = Some(template.to_string());
        self
    }

    pub fn build<R: Runtime>(self) -> TauriPlugin<R> {
        let Builder {
            scheme,
            hosts,
            prefixes,
            hosted,
            hosted_window,
            update_feed,
        } = self;
        PluginBuilder::<R>::new("krizaka")
            .invoke_handler(tauri::generate_handler![
                commands::folders,
                commands::grant_folder,
                commands::revoke_folder,
                commands::list_folder,
                commands::watch_folder,
                commands::unwatch_folder,
                commands::secret_get,
                commands::secret_set,
                commands::secret_remove,
                commands::set_menu,
                commands::take_links,
                commands::update_channel,
                commands::set_update_channel,
            ])
            .setup(move |app, _api| {
                let updater = app.config().plugins.0.get("updater");
                if update_feed.is_some() || updater.is_some() {
                    krizaka_rust::update::require_pubkey(updater)?;
                }
                if let Some(template) = &update_feed {
                    krizaka_rust::update::feed_url(template, Default::default())?;
                }
                let hosted = hosted
                    .iter()
                    .map(|o| nav_guard::normalize_origin(o))
                    .collect::<Result<Vec<_>, _>>()?;
                let data_dir = app.path().app_data_dir()?;
                let scope = ScopeStore::load(&data_dir.join(SCOPE_FILE));
                app.manage(Kit {
                    access: Mutex::new(Access::new(scope)),
                    secrets: Box::new(Checked(krizaka_rust::keychain::Keychain::new(
                        &app.config().identifier,
                    ))),
                    links: LinkRules {
                        scheme,
                        hosts,
                        prefixes,
                    },
                    pending: Mutex::new(vec![]),
                    hosted,
                    hosted_window,
                    update_feed,
                    data_dir,
                    menu_ids: Mutex::new(HashSet::new()),
                    watches: Mutex::new(HashMap::new()),
                });
                app.on_menu_event(|app, event| menu::clicked(app, event.id().as_ref()));
                Ok(())
            })
            .on_navigation(|webview, url| {
                let mut places = own_places(webview.app_handle());
                if let Some(kit) = webview.try_state::<Kit>() {
                    places.extend(kit.hosted.iter().cloned());
                }
                match nav_guard::decide(&places, url) {
                    Navigation::Stay => true,
                    Navigation::Browser => {
                        open_in_browser(url.as_str());
                        false
                    }
                    Navigation::Refuse => false,
                }
            })
            .on_window_ready(|window| {
                let target = window.clone();
                window.on_window_event(move |event| {
                    match event {
                        WindowEvent::DragDrop(DragDropEvent::Enter { .. }) => {
                            let _ = target.emit_to(target.label(), DRAGGING, true);
                        }
                        WindowEvent::DragDrop(DragDropEvent::Leave) => {
                            let _ = target.emit_to(target.label(), DRAGGING, false);
                        }
                        _ => {}
                    }
                    if let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event {
                        let _ = target.emit_to(target.label(), DRAGGING, false);
                        let allowed: Vec<String> = {
                            let kit = target.kit();
                            let mut access = match kit.access.lock() {
                                Ok(a) => a,
                                Err(_) => return,
                            };
                            paths
                                .iter()
                                .filter_map(|p| access.allow_dropped(p))
                                .map(|p| p.display().to_string())
                                .collect()
                        };
                        let _ = target.emit_to(target.label(), DROPPED, allowed);
                    }
                });
            })
            .on_event(|_app, _event| {
                #[cfg(target_os = "macos")]
                if let tauri::RunEvent::Opened { urls } = _event {
                    opened(_app, urls.iter().map(|u| u.as_str()));
                }
            })
            .build()
    }
}

/// The app's own local page, and its dev server while debugging.
fn own_places<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let mut places = vec![
        "tauri://localhost".to_string(),
        "http://tauri.localhost".to_string(),
    ];
    if cfg!(debug_assertions) {
        if let Some(dev) = &app.config().build.dev_url {
            places.push(dev.as_str().to_string());
        }
    }
    places
}

fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = url;
}

/// Links the system opened the app with: each becomes a route sent to the page (and kept until it takes them), the window
/// comes forward; a link that is not the product's is dropped.
fn opened<'a, R: Runtime>(app: &AppHandle<R>, urls: impl Iterator<Item = &'a str>) {
    let Some(kit) = app.try_state::<Kit>() else {
        return;
    };
    let routes: Vec<String> = urls
        .filter_map(|u| deep_link::route(&kit.links, u).ok())
        .collect();
    if routes.is_empty() {
        return;
    }
    if let Ok(mut pending) = kit.pending.lock() {
        pending.extend(routes.iter().cloned());
    }
    let hosted = kit
        .hosted_window
        .as_ref()
        .and_then(|label| Some((app.get_webview_window(label)?, kit.hosted.first()?.clone())));
    if let Some(window) = hosted
        .as_ref()
        .map(|(w, _)| w.clone())
        .or_else(|| app.webview_windows().into_values().next())
    {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let (Some((window, origin)), Some(last)) = (&hosted, routes.last()) {
        if let Ok(target) = url::Url::parse(origin).and_then(|o| o.join(last)) {
            let _ = window.navigate(target);
        }
    }
    for route in routes {
        let _ = app.emit(OPEN, route);
    }
}

/// Sets the menu bar from Rust, for an app whose page cannot send it (a hosted app): the same checks as the command.
pub fn set_menu<R: Runtime>(
    app: &AppHandle<R>,
    spec: &krizaka_rust::menu::MenuSpec,
) -> Result<(), CommandError> {
    commands::apply_menu(app, &app.kit(), spec)
}
