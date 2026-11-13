//! Small secrets of a Mac app (the session token first of all: the bearer the product's API gave at sign in, the same kind
//! the mobile apps hold), in the macOS Keychain — never a file, never the page's storage. The page sees it as the
//! `KeyValueStorage` port of `@krizaka/app-client`, so the Mac apps use the same session store and API client as the
//! mobile ones. This module holds the rules every store follows, and an in-memory store for tests.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

/// Why a secret could not be read, written or removed. A refused key or value is never repeated: the page may have put
/// a secret in the wrong argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The key breaks [`check_key`].
    BadKey,
    /// The value breaks [`check_value`].
    BadValue,
    /// A lock of the store was poisoned by a panic elsewhere.
    Poisoned,
    /// The store itself failed (the Keychain refused, is locked or is missing). The text is the system's, never the
    /// caller's.
    Backend(String),
}

impl SecretError {
    /// The stable code the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            SecretError::BadKey => "secret_bad_key",
            SecretError::BadValue => "secret_bad_value",
            SecretError::Poisoned | SecretError::Backend(_) => "secret_store_failed",
        }
    }
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretError::BadKey => f.write_str("not a secret key"),
            SecretError::BadValue => f.write_str("not a value the Keychain keeps"),
            SecretError::Poisoned => f.write_str("the secret store is locked by a failure"),
            SecretError::Backend(detail) => write!(f, "the secret store failed: {detail}"),
        }
    }
}

impl std::error::Error for SecretError {}

/// The largest value stored; a bearer or a piece of the discovery manifest is far shorter.
// @fixed: the size of a Keychain item, not a setting
pub const MAX_VALUE: usize = 4096;

/// Where the secrets are kept. One Keychain service per app (its bundle id), one account per key.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, value: &str) -> Result<(), SecretError>;
    /// Removing a key that is not there is not an error (signing out twice is signing out).
    fn remove(&self, key: &str) -> Result<(), SecretError>;
}

/// The longest key.
// @fixed: room for `<product>.<name>` of the longest product, not a setting
pub const MAX_KEY: usize = 64;

/// A key is lowercase letters, digits, `.`, `-`, `_`, at most [`MAX_KEY`] characters (`verikora.session`). A refused key is not
/// repeated in the error: the page may have put a secret in the wrong argument.
pub fn check_key(key: &str) -> Result<(), SecretError> {
    let ok = !key.is_empty()
        && key.len() <= MAX_KEY
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err(SecretError::BadKey)
    }
}

/// A value is text of at most [`MAX_VALUE`] bytes, without a NUL.
pub fn check_value(value: &str) -> Result<(), SecretError> {
    if value.len() > MAX_VALUE || value.contains('\0') {
        return Err(SecretError::BadValue);
    }
    Ok(())
}

/// The checked way to every store: what reaches the Keychain has passed [`check_key`] and [`check_value`].
pub struct Checked<S: SecretStore>(pub S);

impl<S: SecretStore> SecretStore for Checked<S> {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        check_key(key)?;
        self.0.get(key)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        check_key(key)?;
        check_value(value)?;
        self.0.set(key, value)
    }

    fn remove(&self, key: &str) -> Result<(), SecretError> {
        check_key(key)?;
        self.0.remove(key)
    }
}

/// A store in memory, for tests and previews.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretError::Poisoned)?
            .get(key)
            .cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Poisoned)?
            .insert(key.into(), value.into());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Poisoned)?
            .remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "yz_sess_3f9a1c0b7e2d4a5f8c6b";

    #[test]
    fn a_session_is_stored_read_and_removed_twice_without_error() {
        let secrets = Checked(MemorySecrets::default());
        assert_eq!(secrets.get("verikora.session").unwrap(), None);
        secrets.set("verikora.session", TOKEN).unwrap();
        assert_eq!(
            secrets.get("verikora.session").unwrap().as_deref(),
            Some(TOKEN)
        );
        secrets
            .set("verikora.discovery.0", "{\"v\": 1, \"services\": []}")
            .unwrap();
        secrets.remove("verikora.session").unwrap();
        secrets.remove("verikora.session").unwrap();
        assert_eq!(secrets.get("verikora.session").unwrap(), None);
        assert!(secrets.get("verikora.discovery.0").unwrap().is_some());
    }

    #[test]
    fn a_bad_key_or_value_never_reaches_the_store() {
        let secrets = Checked(MemorySecrets::default());
        for key in [
            "",
            "Session",
            "a/b",
            "a b",
            "../x",
            &"k".repeat(MAX_KEY + 1),
        ] {
            assert!(secrets.set(key, TOKEN).is_err(), "{key:?}");
            assert!(secrets.get(key).is_err(), "{key:?}");
            assert!(secrets.remove(key).is_err(), "{key:?}");
        }
        assert!(secrets.set("s", &"a".repeat(MAX_VALUE + 1)).is_err());
        assert!(secrets.set("s", "nul\0byte").is_err());
        assert_eq!(secrets.get("s").unwrap(), None);
    }

    #[test]
    fn a_refused_key_or_value_is_not_echoed_in_the_error() -> Result<(), &'static str> {
        let secrets = Checked(MemorySecrets::default());
        let key = format!("Bearer {TOKEN}");
        let refusals = [
            secrets.set(&key, "x").err(),
            secrets.get(&key).err(),
            secrets.remove(&key).err(),
            secrets.set("s", &format!("{TOKEN}\0")).err(),
            secrets.set("s", &TOKEN.repeat(MAX_VALUE)).err(),
        ];
        for refusal in refusals {
            let message = refusal
                .ok_or("a bad key or value was accepted")?
                .to_string();
            assert!(!message.contains(TOKEN), "the input is echoed: {message}");
        }
        Ok(())
    }
}
