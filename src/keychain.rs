//! Small secrets in the macOS Keychain: one generic password per key, service = the app's bundle id, account = the key.
//! Never a file, never the page's storage.

use crate::secrets::{SecretError, SecretStore};
use keyring::Entry;

pub struct Keychain {
    service: String,
}

fn backend(error: keyring::Error) -> SecretError {
    SecretError::Backend(error.to_string())
}

impl Keychain {
    pub fn new(service: &str) -> Self {
        Keychain {
            service: service.to_string(),
        }
    }

    fn entry(&self, key: &str) -> Result<Entry, SecretError> {
        Entry::new(&self.service, key).map_err(backend)
    }
}

impl SecretStore for Keychain {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(backend(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        self.entry(key)?.set_password(value).map_err(backend)
    }

    fn remove(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(backend(e)),
        }
    }
}
