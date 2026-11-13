//! The native menu bar, described by the page (krizaka-ui ADR-002: « desktop is the web in a window »).
//!
//! The page owns the words (its messages, in the person's language) and the shortcuts (the same map `useShortcuts`
//! answers, turned into accelerators with `acceleratorOf` of `@krizaka/ui`). It sends a [`MenuSpec`]; the kit checks it
//! here, then builds the macOS menu: the app menu (about, hide, quit), the Edit menu (without it ⌘C and ⌘V do nothing
//! in a text field), the product's menus, the Window menu. A click on a product item is sent back to the page by its id.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// @fixed: a menu bar a person can read, not a setting
pub const MAX_MENUS: usize = 6;
// @fixed: a menu a person can read, not a setting
pub const MAX_ITEMS: usize = 24;
// @fixed: the width of a menu label, not a setting
pub const MAX_LABEL: usize = 60;

/// The labels of the menus macOS expects, in the page's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StandardLabels {
    pub about: String,
    pub hide: String,
    pub hide_others: String,
    pub show_all: String,
    pub quit: String,
    pub edit: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
    pub close: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemSpec {
    /// Sent back to the page when the item is chosen: lowercase letters, digits, `.`, `-` (`library.add-folder`).
    pub id: String,
    pub label: String,
    /// A Tauri accelerator (`CmdOrCtrl+K`), from `acceleratorOf` of `@krizaka/ui`.
    #[serde(default)]
    pub accelerator: Option<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// A separator before this item.
    #[serde(default)]
    pub separated: bool,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuSpec {
    pub labels: StandardLabels,
    /// The product's own menus, between Edit and Window (File, View, Go…).
    pub menus: Vec<MenuGroupSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuGroupSpec {
    pub title: String,
    pub items: Vec<MenuItemSpec>,
}

/// The shortcuts macOS gives to the standard menus (quit, hide, minimise, close, the Edit menu): never a product's.
pub const RESERVED: &[&str] = &[
    "CmdOrCtrl+Q",
    "CmdOrCtrl+H",
    "Alt+CmdOrCtrl+H",
    "CmdOrCtrl+M",
    "CmdOrCtrl+W",
    "CmdOrCtrl+Z",
    "Shift+CmdOrCtrl+Z",
    "CmdOrCtrl+X",
    "CmdOrCtrl+C",
    "CmdOrCtrl+V",
    "CmdOrCtrl+A",
];

const MODIFIERS: &[&str] = &["CmdOrCtrl", "Cmd", "Ctrl", "Alt", "Shift", "Super"];
const NAMED_KEYS: &[&str] = &[
    "Escape",
    "Enter",
    "Space",
    "Up",
    "Down",
    "Left",
    "Right",
    "Delete",
    "Backspace",
    "Tab",
    "Plus",
];

/// What the page sent that the menu refuses. Standard labels are named by their field; anything else the page wrote
/// (an id, a title, a shortcut) is never repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuError {
    /// A standard label (the field's name) is empty, too long or holds a control character.
    BadStandardLabel(&'static str),
    BadMenuTitle,
    BadItemLabel,
    TooManyMenus,
    /// A menu has no item, or more than [`MAX_ITEMS`].
    BadItemCount,
    BadId,
    DuplicateId,
    EmptyAccelerator,
    BadModifier,
    BadKey,
    /// The shortcut belongs to a standard menu.
    ReservedAccelerator,
    DuplicateAccelerator,
}

impl MenuError {
    /// The stable code the page reads.
    pub fn code(self) -> &'static str {
        match self {
            MenuError::BadStandardLabel(_) => "menu_bad_standard_label",
            MenuError::BadMenuTitle => "menu_bad_menu_title",
            MenuError::BadItemLabel => "menu_bad_item_label",
            MenuError::TooManyMenus => "menu_too_many_menus",
            MenuError::BadItemCount => "menu_bad_item_count",
            MenuError::BadId => "menu_bad_id",
            MenuError::DuplicateId => "menu_duplicate_id",
            MenuError::EmptyAccelerator => "menu_empty_accelerator",
            MenuError::BadModifier => "menu_bad_modifier",
            MenuError::BadKey => "menu_bad_key",
            MenuError::ReservedAccelerator => "menu_reserved_accelerator",
            MenuError::DuplicateAccelerator => "menu_duplicate_accelerator",
        }
    }
}

impl std::fmt::Display for MenuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MenuError::BadStandardLabel(what) => {
                write!(f, "{what}: a label of 1 to {MAX_LABEL} characters")
            }
            MenuError::BadMenuTitle => write!(f, "a menu title of 1 to {MAX_LABEL} characters"),
            MenuError::BadItemLabel => write!(f, "an item label of 1 to {MAX_LABEL} characters"),
            MenuError::TooManyMenus => write!(f, "at most {MAX_MENUS} menus"),
            MenuError::BadItemCount => write!(f, "1 to {MAX_ITEMS} items in a menu"),
            MenuError::BadId => f.write_str("not a menu id"),
            MenuError::DuplicateId => f.write_str("an id is used twice"),
            MenuError::EmptyAccelerator => f.write_str("an empty accelerator"),
            MenuError::BadModifier => f.write_str("not a modifier"),
            MenuError::BadKey => f.write_str("not a key"),
            MenuError::ReservedAccelerator => f.write_str("a shortcut belongs to a standard menu"),
            MenuError::DuplicateAccelerator => f.write_str("a shortcut is used twice"),
        }
    }
}

impl std::error::Error for MenuError {}

/// A label is words a person reads: not empty, not too long, no control character.
fn is_label(label: &str) -> bool {
    let n = label.chars().count();
    !(label.trim().is_empty() || n > MAX_LABEL || label.chars().any(char::is_control))
}

fn check_label(label: &str, error: MenuError) -> Result<(), MenuError> {
    if is_label(label) {
        Ok(())
    } else {
        Err(error)
    }
}

/// An accelerator as `acceleratorOf` writes it: modifiers, then one key (`CmdOrCtrl+Shift+K`, `CmdOrCtrl+,`, `CmdOrCtrl+1`).
pub fn check_accelerator(accelerator: &str) -> Result<(), MenuError> {
    let parts: Vec<&str> = accelerator.split('+').collect();
    let (key, modifiers) = parts.split_last().ok_or(MenuError::EmptyAccelerator)?;
    let mut seen = HashSet::new();
    for m in modifiers {
        if !MODIFIERS.contains(m) || !seen.insert(*m) {
            return Err(MenuError::BadModifier);
        }
    }
    let single = key.chars().count() == 1
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || ",./;'[]-=`".contains(c));
    let function = key.len() >= 2
        && key.starts_with('F')
        && key[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n));
    if !(single || function || NAMED_KEYS.contains(key)) {
        return Err(MenuError::BadKey);
    }
    Ok(())
}

fn check_id(id: &str) -> Result<(), MenuError> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(MenuError::BadId)
    }
}

impl MenuSpec {
    /// Everything the page sent, checked before a native menu is built from it.
    pub fn validate(&self) -> Result<(), MenuError> {
        let l = &self.labels;
        for (label, what) in [
            (&l.about, "about"),
            (&l.hide, "hide"),
            (&l.hide_others, "hideOthers"),
            (&l.show_all, "showAll"),
            (&l.quit, "quit"),
            (&l.edit, "edit"),
            (&l.undo, "undo"),
            (&l.redo, "redo"),
            (&l.cut, "cut"),
            (&l.copy, "copy"),
            (&l.paste, "paste"),
            (&l.select_all, "selectAll"),
            (&l.window, "window"),
            (&l.minimize, "minimize"),
            (&l.zoom, "zoom"),
            (&l.close, "close"),
        ] {
            check_label(label, MenuError::BadStandardLabel(what))?;
        }
        if self.menus.len() > MAX_MENUS {
            return Err(MenuError::TooManyMenus);
        }
        let mut ids = HashSet::new();
        let mut accelerators = HashSet::new();
        for menu in &self.menus {
            check_label(&menu.title, MenuError::BadMenuTitle)?;
            if menu.items.is_empty() || menu.items.len() > MAX_ITEMS {
                return Err(MenuError::BadItemCount);
            }
            for item in &menu.items {
                check_id(&item.id)?;
                check_label(&item.label, MenuError::BadItemLabel)?;
                if !ids.insert(item.id.as_str()) {
                    return Err(MenuError::DuplicateId);
                }
                if let Some(a) = &item.accelerator {
                    check_accelerator(a)?;
                    if RESERVED.contains(&a.as_str()) {
                        return Err(MenuError::ReservedAccelerator);
                    }
                    if !accelerators.insert(a.as_str()) {
                        return Err(MenuError::DuplicateAccelerator);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> StandardLabels {
        let w = |s: &str| s.to_string();
        StandardLabels {
            about: w("À propos de Verikora"),
            hide: w("Masquer Verikora"),
            hide_others: w("Masquer les autres"),
            show_all: w("Tout afficher"),
            quit: w("Quitter Verikora"),
            edit: w("Édition"),
            undo: w("Annuler"),
            redo: w("Rétablir"),
            cut: w("Couper"),
            copy: w("Copier"),
            paste: w("Coller"),
            select_all: w("Tout sélectionner"),
            window: w("Fenêtre"),
            minimize: w("Réduire"),
            zoom: w("Zoom"),
            close: w("Fermer"),
        }
    }

    fn item(id: &str, accelerator: Option<&str>) -> MenuItemSpec {
        MenuItemSpec {
            id: id.into(),
            label: "Ajouter un dossier…".into(),
            accelerator: accelerator.map(Into::into),
            enabled: true,
            separated: false,
        }
    }

    fn spec(items: Vec<MenuItemSpec>) -> MenuSpec {
        MenuSpec {
            labels: labels(),
            menus: vec![MenuGroupSpec {
                title: "Fichier".into(),
                items,
            }],
        }
    }

    #[test]
    fn a_menu_as_the_page_sends_it_is_accepted() {
        let s = spec(vec![
            item("library.add-folder", Some("CmdOrCtrl+O")),
            item("check.new", Some("CmdOrCtrl+N")),
            item("go.reports", Some("CmdOrCtrl+2")),
            item("app.settings", Some("CmdOrCtrl+,")),
            item("app.search", Some("Shift+CmdOrCtrl+K")),
            item("view.help", None),
        ]);
        assert_eq!(s.validate(), Ok(()));
        let from_page: MenuSpec = serde_json::from_value(serde_json::json!({
            "labels": serde_json::to_value(labels()).unwrap(),
            "menus": [{ "title": "Fichier", "items": [{ "id": "check.new", "label": "Nouvelle vérification", "accelerator": "CmdOrCtrl+N" }] }]
        }))
        .unwrap();
        assert!(from_page.menus[0].items[0].enabled);
        assert_eq!(from_page.validate(), Ok(()));
    }

    #[test]
    fn accelerators_are_the_ones_accelerator_of_writes() {
        for ok in [
            "CmdOrCtrl+K",
            "Shift+CmdOrCtrl+/",
            "Alt+Up",
            "CmdOrCtrl+Enter",
            "F5",
            "CmdOrCtrl+Plus",
        ] {
            assert_eq!(check_accelerator(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "CmdOrCtrl+",
            "Hyper+K",
            "CmdOrCtrl+CmdOrCtrl+K",
            "CmdOrCtrl+k",
            "CmdOrCtrl+KK",
            "F13",
            "CmdOrCtrl+é",
        ] {
            assert!(check_accelerator(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn refuses_bad_ids_twice_used_ids_or_shortcuts_and_unreadable_labels() {
        assert!(spec(vec![item("Check.New", None)]).validate().is_err());
        assert!(spec(vec![item("check new", None)]).validate().is_err());
        assert!(spec(vec![item("a", None), item("a", None)])
            .validate()
            .is_err());
        assert!(spec(vec![
            item("a", Some("CmdOrCtrl+N")),
            item("b", Some("CmdOrCtrl+N"))
        ])
        .validate()
        .is_err());
        assert!(spec(vec![]).validate().is_err());
        for standard in ["CmdOrCtrl+Q", "CmdOrCtrl+C", "Shift+CmdOrCtrl+Z"] {
            assert!(
                spec(vec![item("a", Some(standard))]).validate().is_err(),
                "{standard}"
            );
        }
        let mut long = item("a", None);
        long.label = "x".repeat(MAX_LABEL + 1);
        assert!(spec(vec![long]).validate().is_err());
        let mut blank = spec(vec![item("a", None)]);
        blank.labels.copy = " ".into();
        assert!(blank.validate().is_err());
        let mut many = spec(vec![item("a", None)]);
        many.menus = (0..=MAX_MENUS)
            .map(|i| MenuGroupSpec {
                title: format!("M{i}"),
                items: vec![item(&format!("m{i}"), None)],
            })
            .collect();
        assert!(many.validate().is_err());
    }
}
