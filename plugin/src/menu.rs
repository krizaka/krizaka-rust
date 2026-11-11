//! Builds the macOS menu bar from a checked [`MenuSpec`]: the app menu, Edit, the product's menus, Window.

use crate::{KitExt, MENU};
use krizaka_rust::menu::MenuSpec;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

pub fn build<R: Runtime>(app: &AppHandle<R>, spec: &MenuSpec) -> tauri::Result<Menu<R>> {
    let l = &spec.labels;
    let name = app.package_info().name.clone();
    let menu = Menu::new(app)?;
    menu.append(&Submenu::with_items(
        app,
        &name,
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&l.about), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some(&l.hide))?,
            &PredefinedMenuItem::hide_others(app, Some(&l.hide_others))?,
            &PredefinedMenuItem::show_all(app, Some(&l.show_all))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(&l.quit))?,
        ],
    )?)?;
    menu.append(&Submenu::with_items(
        app,
        &l.edit,
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(&l.undo))?,
            &PredefinedMenuItem::redo(app, Some(&l.redo))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(&l.cut))?,
            &PredefinedMenuItem::copy(app, Some(&l.copy))?,
            &PredefinedMenuItem::paste(app, Some(&l.paste))?,
            &PredefinedMenuItem::select_all(app, Some(&l.select_all))?,
        ],
    )?)?;
    for group in &spec.menus {
        let sub = Submenu::new(app, &group.title, true)?;
        for item in &group.items {
            if item.separated {
                sub.append(&PredefinedMenuItem::separator(app)?)?;
            }
            sub.append(&MenuItem::with_id(
                app,
                item.id.clone(),
                &item.label,
                item.enabled,
                item.accelerator.as_deref(),
            )?)?;
        }
        menu.append(&sub)?;
    }
    let window = Submenu::with_items(
        app,
        &l.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(&l.minimize))?,
            &PredefinedMenuItem::maximize(app, Some(&l.zoom))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some(&l.close))?,
        ],
    )?;
    #[cfg(target_os = "macos")]
    window.set_as_windows_menu_for_nsapp()?;
    menu.append(&window)?;
    Ok(menu)
}

/// A click on one of the product's items goes to the page; anything else (a standard item) is the system's.
pub fn clicked<R: Runtime>(app: &AppHandle<R>, id: &str) {
    let known = app
        .kit()
        .menu_ids
        .lock()
        .map(|ids| ids.contains(id))
        .unwrap_or(false);
    if known {
        let _ = app.emit(MENU, id.to_string());
    }
}
