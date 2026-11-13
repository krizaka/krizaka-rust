//! What a command answers when it fails: a `code` the page branches on, and a fixed `message` for a person.
//!
//! On the wire: `{"code": "scope_outside", "message": "…"}`. The **code is the contract**: snake_case, listed once in
//! [`CODES`], and never renamed or reused for another meaning (a new failure gets a new code). The message is a sentence
//! for a log or a fallback; it never repeats what the page sent (a path, a key, a value may hold a secret).

use krizaka_rust::files::FilesError;
use krizaka_rust::menu::MenuError;
use krizaka_rust::nav_guard::OriginError;
use krizaka_rust::scope::ScopeError;
use krizaka_rust::secrets::SecretError;
use krizaka_rust::settings::SettingsError;
use krizaka_rust::update::{FeedError, MissingPubkey};
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use std::fmt;
use std::sync::{Mutex, MutexGuard};

/// Every code a command may answer, with its message. The one place to read them (and to document them for the page).
pub const CODES: &[(&str, &str)] = &[
    (
        "scope_forbidden",
        "this folder is too broad or too sensitive to be granted",
    ),
    ("scope_not_found", "the path does not exist"),
    ("scope_outside", "the path is outside the granted folders"),
    ("scope_io", "the folder grants could not be read or saved"),
    ("files_io", "the folder or file could not be read"),
    ("files_not_a_file", "this is not a file"),
    ("secret_bad_key", "not a secret key"),
    ("secret_bad_value", "not a value the Keychain keeps"),
    ("secret_store_failed", "the secret store failed"),
    ("settings_no_folder", "the settings path has no folder"),
    (
        "settings_not_a_file_name",
        "the settings path is not a file name",
    ),
    ("settings_io", "the settings file could not be written"),
    ("settings_encode", "the settings could not be encoded"),
    (
        "menu_bad_standard_label",
        "a standard menu label is not valid",
    ),
    ("menu_bad_menu_title", "a menu title is not valid"),
    ("menu_bad_item_label", "a menu item label is not valid"),
    ("menu_too_many_menus", "too many menus"),
    ("menu_bad_item_count", "a menu has no item or too many"),
    ("menu_bad_id", "a menu item id is not valid"),
    ("menu_duplicate_id", "a menu item id is used twice"),
    ("menu_empty_accelerator", "a shortcut is empty"),
    (
        "menu_bad_modifier",
        "a shortcut has a modifier that is not known",
    ),
    ("menu_bad_key", "a shortcut has a key that is not known"),
    (
        "menu_reserved_accelerator",
        "a shortcut belongs to a standard menu",
    ),
    ("menu_duplicate_accelerator", "a shortcut is used twice"),
    (
        "feed_missing_channel",
        "the update feed must name {channel}",
    ),
    ("feed_not_a_url", "the update feed is not a valid URL"),
    (
        "feed_not_https",
        "the update feed must be served over https",
    ),
    ("feed_credentials", "no credentials in the update feed"),
    ("updater_missing_pubkey", "the updater needs a public key"),
    ("origin_not_a_url", "the hosted app is not a valid URL"),
    ("origin_credentials", "no credentials in the hosted app URL"),
    ("origin_missing_host", "the hosted app URL has no host"),
    (
        "origin_not_https",
        "the hosted app must be served over https",
    ),
    ("not_a_folder", "this is not a folder"),
    ("watch_failed", "the folder could not be watched"),
    ("platform_failed", "the system refused the operation"),
    (
        "kit_locked",
        "the kit's state is locked by an earlier failure",
    ),
];

/// A command's failure.
#[derive(Debug)]
pub enum CommandError {
    Scope(ScopeError),
    Files(FilesError),
    Secret(SecretError),
    Settings(SettingsError),
    Menu(MenuError),
    Feed(FeedError),
    Pubkey(MissingPubkey),
    Origin(OriginError),
    /// The path is not a folder (watching asks for one).
    NotAFolder,
    /// The system's file watcher failed.
    Watch(notify::Error),
    /// The window system (menu, picker) failed; the text is the system's.
    Platform(String),
    /// A lock of the kit's state was poisoned by a panic elsewhere.
    Locked,
}

impl CommandError {
    /// The stable code the page reads; always one of [`CODES`].
    pub fn code(&self) -> &'static str {
        match self {
            CommandError::Scope(e) => e.code(),
            CommandError::Files(e) => e.code(),
            CommandError::Secret(e) => e.code(),
            CommandError::Settings(e) => e.code(),
            CommandError::Menu(e) => e.code(),
            CommandError::Feed(e) => e.code(),
            CommandError::Pubkey(e) => e.code(),
            CommandError::Origin(e) => e.code(),
            CommandError::NotAFolder => "not_a_folder",
            CommandError::Watch(_) => "watch_failed",
            CommandError::Platform(_) => "platform_failed",
            CommandError::Locked => "kit_locked",
        }
    }

    /// The fixed sentence of [`CommandError::code`].
    pub fn message(&self) -> &'static str {
        let code = self.code();
        CODES
            .iter()
            .find(|(known, _)| *known == code)
            .map_or("the command failed", |(_, message)| message)
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CommandError::Scope(e) => Some(e),
            CommandError::Files(e) => Some(e),
            CommandError::Secret(e) => Some(e),
            CommandError::Settings(e) => Some(e),
            CommandError::Menu(e) => Some(e),
            CommandError::Feed(e) => Some(e),
            CommandError::Pubkey(e) => Some(e),
            CommandError::Origin(e) => Some(e),
            CommandError::Watch(e) => Some(e),
            CommandError::NotAFolder | CommandError::Platform(_) | CommandError::Locked => None,
        }
    }
}

impl Serialize for CommandError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("CommandError", 2)?;
        wire.serialize_field("code", self.code())?;
        wire.serialize_field("message", self.message())?;
        wire.end()
    }
}

macro_rules! from_error {
    ($($source:ty => $variant:ident),+ $(,)?) => {
        $(impl From<$source> for CommandError {
            fn from(error: $source) -> Self {
                CommandError::$variant(error)
            }
        })+
    };
}

from_error! {
    ScopeError => Scope,
    FilesError => Files,
    SecretError => Secret,
    SettingsError => Settings,
    MenuError => Menu,
    FeedError => Feed,
    MissingPubkey => Pubkey,
    OriginError => Origin,
    notify::Error => Watch,
}

/// Locks a mutex of the kit's state; a poisoned one is [`CommandError::Locked`], not a panic.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, CommandError> {
    mutex.lock().map_err(|_| CommandError::Locked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn one_of_each() -> Vec<CommandError> {
        let encode = <serde_json::Error as serde::ser::Error>::custom("x");
        vec![
            ScopeError::Forbidden("/".into()).into(),
            ScopeError::NotFound("/x".into()).into(),
            ScopeError::OutsideScope("/x".into()).into(),
            ScopeError::Io("x".into()).into(),
            FilesError::Io(std::io::Error::other("x")).into(),
            FilesError::NotAFile.into(),
            FilesError::Scope(ScopeError::Io("x".into())).into(),
            SecretError::BadKey.into(),
            SecretError::BadValue.into(),
            SecretError::Poisoned.into(),
            SecretError::Backend("x".into()).into(),
            SettingsError::NoFolder.into(),
            SettingsError::NotAFileName.into(),
            SettingsError::Io(std::io::Error::other("x")).into(),
            SettingsError::Encode(encode).into(),
            MenuError::BadStandardLabel("copy").into(),
            MenuError::BadMenuTitle.into(),
            MenuError::BadItemLabel.into(),
            MenuError::TooManyMenus.into(),
            MenuError::BadItemCount.into(),
            MenuError::BadId.into(),
            MenuError::DuplicateId.into(),
            MenuError::EmptyAccelerator.into(),
            MenuError::BadModifier.into(),
            MenuError::BadKey.into(),
            MenuError::ReservedAccelerator.into(),
            MenuError::DuplicateAccelerator.into(),
            FeedError::MissingChannel.into(),
            FeedError::NotAUrl.into(),
            FeedError::NotHttps.into(),
            FeedError::Credentials.into(),
            MissingPubkey.into(),
            OriginError::NotAUrl.into(),
            OriginError::Credentials.into(),
            OriginError::MissingHost.into(),
            OriginError::NotHttps.into(),
            CommandError::NotAFolder,
            CommandError::Watch(notify::Error::generic("x")),
            CommandError::Platform("x".into()),
            CommandError::Locked,
        ]
    }

    #[test]
    fn a_command_error_is_a_code_and_a_message_on_the_wire() -> TestResult {
        let error: CommandError = ScopeError::OutsideScope("/Users/a/secret.pdf".into()).into();
        assert_eq!(
            serde_json::to_value(&error)?,
            json!({"code": "scope_outside", "message": "the path is outside the granted folders"})
        );
        let error: CommandError = SecretError::BadKey.into();
        assert_eq!(
            serde_json::to_value(&error)?,
            json!({"code": "secret_bad_key", "message": "not a secret key"})
        );
        Ok(())
    }

    #[test]
    fn every_code_is_listed_once_in_snake_case_and_has_a_message() {
        let mut seen = std::collections::HashSet::new();
        for (code, message) in CODES {
            assert!(seen.insert(*code), "{code} is listed twice");
            assert!(
                !code.is_empty()
                    && code.starts_with(|c: char| c.is_ascii_lowercase())
                    && code
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{code} is not snake_case"
            );
            assert!(!message.is_empty());
        }
        for error in one_of_each() {
            assert!(
                seen.contains(error.code()),
                "{} is not in CODES",
                error.code()
            );
        }
    }

    #[test]
    fn the_message_never_repeats_what_the_page_sent() -> TestResult {
        for error in one_of_each() {
            let wire = serde_json::to_string(&error)?;
            for secret in ["/Users/a/secret.pdf", "/x", "yz_sess_token"] {
                assert!(!wire.contains(secret), "{wire}");
            }
        }
        let error: CommandError = ScopeError::Forbidden("/Users/a/.ssh".into()).into();
        assert!(!serde_json::to_string(&error)?.contains(".ssh"));
        Ok(())
    }

    #[test]
    fn a_command_error_is_a_std_error_with_its_cause() {
        fn is_error<E: std::error::Error + Send + Sync + 'static>() {}
        is_error::<CommandError>();
        let error: CommandError = FilesError::NotAFile.into();
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn a_poisoned_lock_is_an_error_not_a_panic() {
        let mutex = Mutex::new(1);
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _guard = mutex.lock().unwrap();
                panic!("poison it");
            })
            .join()
        });
        assert!(matches!(lock(&mutex), Err(CommandError::Locked)));
    }
}
