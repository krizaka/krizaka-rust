//! OAuth device flow (RFC 8628) for a Mac app that signs in to a provider (GitHub first: a GitHub App's user-to-server
//! token), with no client secret on the Mac. The rules live here, with no network: the app gives a [`Transport`] (one
//! form POST that answers JSON), and the tokens are kept through [`crate::secrets::SecretStore`] (the Keychain), on the
//! Rust side only — the page sees [`Session`] (connected as whom), never a token.
//!
//! Steps: [`start`] asks a code; the app shows [`DeviceCode::user_code`] and [`DeviceCode::verification_uri`];
//! [`wait`] polls (honouring `slow_down`, stopping on `expired_token` / `access_denied`); [`TokenVault::save`] keeps the
//! tokens; [`TokenVault::access_token`] returns a valid one, refreshing it when it is about to expire;
//! [`TokenVault::sign_out`] removes both secrets (revoking at the provider is the person's, in its settings).

use crate::secrets::{SecretError, SecretStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// The app's HTTP client could not do the request (offline, refused, not JSON…). The text is the client's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError(pub String);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the request failed: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

/// One form POST that answers JSON (`Accept: application/json`). The app supplies it (ureq, reqwest…); tests mock it.
pub trait Transport {
    fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<Value, TransportError>;
}

/// A provider's two endpoints and the app's public client id (no secret: the device flow has none).
#[derive(Debug, Clone)]
pub struct Provider {
    pub device_code_url: String,
    pub token_url: String,
    pub client_id: String,
}

impl Provider {
    /// GitHub (github.com); a firm on GitHub Enterprise builds its own `Provider` with its host.
    pub fn github(client_id: &str) -> Self {
        Provider {
            device_code_url: "https://github.com/login/device/code".into(),
            token_url: "https://github.com/login/oauth/access_token".into(),
            client_id: client_id.into(),
        }
    }
}

/// What the person sees: the code to type and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    #[serde(skip_serializing)]
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// The tokens, with the instant (unix seconds) the access token stops being valid; the refresh token is optional (a
/// provider without expiring tokens gives none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
}

/// How the flow ends when it is not a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Expired,
    Denied,
    /// The request could not be made.
    Transport(TransportError),
    /// The provider refused for another reason: its `error` code.
    Provider(String),
    /// The provider's answer lacks what the flow needs (the name of what is missing).
    Malformed(&'static str),
}

impl Failure {
    /// The stable code the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            Failure::Expired => "device_expired",
            Failure::Denied => "device_denied",
            Failure::Transport(_) => "device_transport",
            Failure::Provider(_) => "device_provider",
            Failure::Malformed(_) => "device_malformed",
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Expired => write!(f, "the code expired, start again"),
            Failure::Denied => write!(f, "the request was denied"),
            Failure::Transport(e) => write!(f, "{e}"),
            Failure::Provider(code) => write!(f, "the provider refused: {code}"),
            Failure::Malformed(what) => write!(f, "the provider's answer lacks {what}"),
        }
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Failure::Transport(e) => Some(e),
            _ => None,
        }
    }
}

impl From<TransportError> for Failure {
    fn from(error: TransportError) -> Self {
        Failure::Transport(error)
    }
}

/// Why the token vault failed: the secret store, or the refresh the vault tried.
#[derive(Debug)]
pub enum VaultError {
    Secrets(SecretError),
    Refresh(Failure),
}

impl VaultError {
    /// The stable code the page reads: the one of the failure underneath.
    pub fn code(&self) -> &'static str {
        match self {
            VaultError::Secrets(e) => e.code(),
            VaultError::Refresh(f) => f.code(),
        }
    }
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::Secrets(e) => write!(f, "{e}"),
            VaultError::Refresh(e) => write!(f, "the refresh failed: {e}"),
        }
    }
}

impl std::error::Error for VaultError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VaultError::Secrets(e) => Some(e),
            VaultError::Refresh(e) => Some(e),
        }
    }
}

impl From<SecretError> for VaultError {
    fn from(error: SecretError) -> Self {
        VaultError::Secrets(error)
    }
}

/// The smallest wait between two polls, whatever the provider says (it asks 5 s).
// @fixed: RFC 8628 §3.5 default interval
pub const DEFAULT_INTERVAL: u64 = 5;
/// Added to the interval on `slow_down`.
// @fixed: RFC 8628 §3.5
pub const SLOW_DOWN_STEP: u64 = 5;
/// How long a device code lives when the provider does not say.
// @fixed: RFC 8628 §3.2 lets the provider choose; fifteen minutes is the usual window to type a code
pub const DEFAULT_EXPIRES_IN: u64 = 900;
/// A token is refreshed this many seconds before it expires.
// @fixed: a margin for the clock and the round trip, not a business setting
pub const REFRESH_MARGIN: u64 = 60;

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Step 1: ask the provider for a device and user code.
pub fn start(t: &dyn Transport, p: &Provider, scope: &str) -> Result<DeviceCode, Failure> {
    let mut fields = vec![("client_id", p.client_id.as_str())];
    if !scope.is_empty() {
        fields.push(("scope", scope));
    }
    let v = t.post_form(&p.device_code_url, &fields)?;
    if let Some(e) = text(&v, "error") {
        return Err(Failure::Provider(e));
    }
    let missing = Failure::Malformed;
    Ok(DeviceCode {
        device_code: text(&v, "device_code").ok_or_else(|| missing("device_code"))?,
        user_code: text(&v, "user_code").ok_or_else(|| missing("user_code"))?,
        verification_uri: text(&v, "verification_uri")
            .ok_or_else(|| missing("verification_uri"))?,
        expires_in: v
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_EXPIRES_IN),
        interval: v
            .get("interval")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_INTERVAL)
            .max(1),
    })
}

fn tokens_from(v: &Value, now: u64) -> Option<Tokens> {
    Some(Tokens {
        access_token: text(v, "access_token")?,
        refresh_token: text(v, "refresh_token"),
        expires_at: v.get("expires_in").and_then(Value::as_u64).map(|s| now + s),
    })
}

/// One poll of the token endpoint: `Ok(None)` = keep waiting (the second value is the new interval).
fn poll_once(
    t: &dyn Transport,
    p: &Provider,
    code: &DeviceCode,
    interval: u64,
    now: u64,
) -> Result<Result<Tokens, u64>, Failure> {
    let v = t.post_form(
        &p.token_url,
        &[
            ("client_id", p.client_id.as_str()),
            ("device_code", code.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
    )?;
    if let Some(tokens) = tokens_from(&v, now) {
        return Ok(Ok(tokens));
    }
    match text(&v, "error").as_deref() {
        Some("authorization_pending") => Ok(Err(interval)),
        Some("slow_down") => Ok(Err(interval + SLOW_DOWN_STEP)),
        Some("expired_token") => Err(Failure::Expired),
        Some("access_denied") => Err(Failure::Denied),
        Some(other) => Err(Failure::Provider(other.into())),
        None => Err(Failure::Malformed("a token or an error")),
    }
}

/// Step 2: poll until the person approves, the code expires or the request is denied. `sleep` and `now` are the app's
/// (tests pass a recorder and a counter), `cancelled` lets the window close the wait.
pub fn wait(
    t: &dyn Transport,
    p: &Provider,
    code: &DeviceCode,
    sleep: &mut dyn FnMut(Duration),
    now: &dyn Fn() -> u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Tokens, Failure> {
    let deadline = now() + code.expires_in;
    let mut interval = code.interval;
    loop {
        sleep(Duration::from_secs(interval));
        if cancelled() {
            return Err(Failure::Denied);
        }
        if now() >= deadline {
            return Err(Failure::Expired);
        }
        match poll_once(t, p, code, interval, now())? {
            Ok(tokens) => return Ok(tokens),
            Err(next) => interval = next,
        }
    }
}

/// Exchange a refresh token for a new pair (the provider rotates the refresh token: the old one is spent).
pub fn refresh(
    t: &dyn Transport,
    p: &Provider,
    refresh_token: &str,
    now: u64,
) -> Result<Tokens, Failure> {
    let v = t.post_form(
        &p.token_url,
        &[
            ("client_id", p.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )?;
    tokens_from(&v, now).ok_or_else(|| match text(&v, "error") {
        Some(code) => Failure::Provider(code),
        None => Failure::Malformed("a token or an error"),
    })
}

/// What the page may know: whether the person is connected, never a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub connected: bool,
}

/// The tokens in a [`SecretStore`] (the Keychain), under `<prefix>.access` / `.refresh` / `.expires`.
pub struct TokenVault<'a> {
    store: &'a dyn SecretStore,
    prefix: String,
}

impl<'a> TokenVault<'a> {
    pub fn new(store: &'a dyn SecretStore, prefix: &str) -> Self {
        TokenVault {
            store,
            prefix: prefix.into(),
        }
    }

    fn key(&self, part: &str) -> String {
        format!("{}.{part}", self.prefix)
    }

    pub fn save(&self, tokens: &Tokens) -> Result<(), SecretError> {
        self.store.set(&self.key("access"), &tokens.access_token)?;
        match &tokens.refresh_token {
            Some(r) => self.store.set(&self.key("refresh"), r)?,
            None => self.store.remove(&self.key("refresh"))?,
        }
        match tokens.expires_at {
            Some(e) => self.store.set(&self.key("expires"), &e.to_string())?,
            None => self.store.remove(&self.key("expires"))?,
        }
        Ok(())
    }

    pub fn load(&self) -> Result<Option<Tokens>, SecretError> {
        let Some(access_token) = self.store.get(&self.key("access"))? else {
            return Ok(None);
        };
        Ok(Some(Tokens {
            access_token,
            refresh_token: self.store.get(&self.key("refresh"))?,
            expires_at: self
                .store
                .get(&self.key("expires"))?
                .and_then(|s| s.parse().ok()),
        }))
    }

    pub fn session(&self) -> Result<Session, SecretError> {
        Ok(Session {
            connected: self.load()?.is_some(),
        })
    }

    /// Removes both secrets at once; signing out twice is signing out.
    pub fn sign_out(&self) -> Result<(), SecretError> {
        for part in ["access", "refresh", "expires"] {
            self.store.remove(&self.key(part))?;
        }
        Ok(())
    }

    /// A valid access token: the stored one, or a refreshed one when it expires within [`REFRESH_MARGIN`]. `Ok(None)`
    /// = not connected, or the refresh was refused (the person signs in again; the dead tokens are removed).
    pub fn access_token(
        &self,
        t: &dyn Transport,
        p: &Provider,
        now: u64,
    ) -> Result<Option<String>, VaultError> {
        let Some(tokens) = self.load()? else {
            return Ok(None);
        };
        let fresh = tokens.expires_at.map_or(true, |e| e > now + REFRESH_MARGIN);
        if fresh {
            return Ok(Some(tokens.access_token));
        }
        let Some(refresh_token) = tokens.refresh_token else {
            self.sign_out()?;
            return Ok(None);
        };
        match refresh(t, p, &refresh_token, now) {
            Ok(new) => {
                self.save(&new)?;
                Ok(Some(new.access_token))
            }
            Err(Failure::Provider(m))
                if m.starts_with("bad_refresh_token") || m == "incorrect_client_credentials" =>
            {
                self.sign_out()?;
                Ok(None)
            }
            Err(f) => Err(VaultError::Refresh(f)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{Checked, MemorySecrets};
    use serde_json::json;
    use std::cell::{Cell, RefCell};

    type Call = (String, Vec<(String, String)>);

    /// A mocked provider: answers are queued, requests are recorded.
    struct Mock {
        answers: RefCell<Vec<Result<Value, TransportError>>>,
        calls: RefCell<Vec<Call>>,
    }

    impl Mock {
        fn new(answers: Vec<Result<Value, TransportError>>) -> Self {
            Mock {
                answers: RefCell::new(answers.into_iter().rev().collect()),
                calls: RefCell::new(vec![]),
            }
        }
    }

    impl Transport for Mock {
        fn post_form(&self, url: &str, fields: &[(&str, &str)]) -> Result<Value, TransportError> {
            self.calls.borrow_mut().push((
                url.into(),
                fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            ));
            self.answers.borrow_mut().pop().expect("no more answers")
        }
    }

    fn provider() -> Provider {
        Provider::github("Iv-test-client")
    }

    fn code_answer() -> Value {
        json!({"device_code": "dc-1", "user_code": "WDJB-MJHT", "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 5})
    }

    fn token_answer() -> Value {
        json!({"access_token": "ghu_fake", "refresh_token": "ghr_fake", "expires_in": 28800})
    }

    fn pending() -> Result<Value, TransportError> {
        Ok(json!({"error": "authorization_pending"}))
    }

    #[test]
    fn a_failure_is_a_std_error_with_a_stable_code_and_its_cause() {
        fn is_error<E: std::error::Error + Send + Sync + 'static>() {}
        is_error::<Failure>();
        is_error::<VaultError>();
        let transport = Failure::Transport(TransportError("offline".into()));
        assert!(std::error::Error::source(&transport).is_some());
        assert!(std::error::Error::source(&Failure::Expired).is_none());
        let codes: Vec<_> = [
            Failure::Expired,
            Failure::Denied,
            transport,
            Failure::Provider("x".into()),
            Failure::Malformed("x"),
        ]
        .iter()
        .map(Failure::code)
        .collect();
        assert_eq!(
            codes,
            [
                "device_expired",
                "device_denied",
                "device_transport",
                "device_provider",
                "device_malformed"
            ]
        );
    }

    #[test]
    fn start_reads_the_code_to_show_and_sends_only_the_public_client_id() {
        let t = Mock::new(vec![Ok(code_answer())]);
        let c = start(&t, &provider(), "").unwrap();
        assert_eq!(c.user_code, "WDJB-MJHT");
        assert_eq!(c.verification_uri, "https://github.com/login/device");
        let calls = t.calls.borrow();
        assert_eq!(calls[0].0, "https://github.com/login/device/code");
        assert_eq!(
            calls[0].1,
            vec![("client_id".to_string(), "Iv-test-client".to_string())]
        );
        // the device code never reaches the page
        assert!(!serde_json::to_string(&c).unwrap().contains("dc-1"));
    }

    #[test]
    fn start_refuses_an_incomplete_or_errored_answer() {
        let t = Mock::new(vec![
            Ok(json!({"user_code": "X"})),
            Ok(json!({"error": "device_flow_disabled"})),
        ]);
        assert!(start(&t, &provider(), "").is_err());
        assert_eq!(
            start(&t, &provider(), "").unwrap_err(),
            Failure::Provider("device_flow_disabled".into())
        );
    }

    #[test]
    fn wait_polls_at_the_interval_slows_down_and_returns_the_tokens() {
        let t = Mock::new(vec![
            Ok(code_answer()),
            pending(),
            Ok(json!({"error": "slow_down"})),
            Ok(token_answer()),
        ]);
        let c = start(&t, &provider(), "").unwrap();
        let slept = RefCell::new(vec![]);
        let clock = Cell::new(1000);
        let tokens = wait(
            &t,
            &provider(),
            &c,
            &mut |d| {
                slept.borrow_mut().push(d.as_secs());
                clock.set(clock.get() + d.as_secs());
            },
            &|| clock.get(),
            &|| false,
        )
        .unwrap();
        assert_eq!(*slept.borrow(), vec![5, 5, 10]);
        assert_eq!(tokens.access_token, "ghu_fake");
        assert_eq!(tokens.refresh_token.as_deref(), Some("ghr_fake"));
        assert_eq!(tokens.expires_at, Some(1020 + 28800));
        let calls = t.calls.borrow();
        assert!(calls[1].1.contains(&(
            "grant_type".into(),
            "urn:ietf:params:oauth:grant-type:device_code".into()
        )));
    }

    #[test]
    fn wait_ends_on_expiry_denial_cancel_and_unknown_errors() {
        for (answer, want) in [
            (Ok(json!({"error": "expired_token"})), Failure::Expired),
            (Ok(json!({"error": "access_denied"})), Failure::Denied),
            (
                Ok(json!({"error": "unsupported_grant_type"})),
                Failure::Provider("unsupported_grant_type".into()),
            ),
            (
                Err(TransportError("offline".into())),
                Failure::Transport(TransportError("offline".into())),
            ),
        ] {
            let t = Mock::new(vec![Ok(code_answer()), answer]);
            let c = start(&t, &provider(), "").unwrap();
            let r = wait(&t, &provider(), &c, &mut |_| {}, &|| 0, &|| false);
            assert_eq!(r.unwrap_err(), want);
        }
        // closed window: no poll at all
        let t = Mock::new(vec![Ok(code_answer())]);
        let c = start(&t, &provider(), "").unwrap();
        assert_eq!(
            wait(&t, &provider(), &c, &mut |_| {}, &|| 0, &|| true).unwrap_err(),
            Failure::Denied
        );
        assert_eq!(t.calls.borrow().len(), 1);
        // local deadline passed
        let t = Mock::new(vec![Ok(code_answer())]);
        let c = start(&t, &provider(), "").unwrap();
        let clock = Cell::new(0);
        let r = wait(
            &t,
            &provider(),
            &c,
            &mut |_| clock.set(clock.get() + 1000),
            &|| clock.get(),
            &|| false,
        );
        assert_eq!(r.unwrap_err(), Failure::Expired);
    }

    fn vault_with(store: &Checked<MemorySecrets>, expires_at: Option<u64>, refresh: Option<&str>) {
        TokenVault::new(store, "gh")
            .save(&Tokens {
                access_token: "ghu_old".into(),
                refresh_token: refresh.map(str::to_owned),
                expires_at,
            })
            .unwrap();
    }

    #[test]
    fn a_valid_token_is_returned_and_an_expiring_one_is_refreshed_and_rotated() {
        let store = Checked(MemorySecrets::default());
        let vault = TokenVault::new(&store, "gh");
        assert_eq!(
            vault
                .access_token(&Mock::new(vec![]), &provider(), 0)
                .unwrap(),
            None
        );
        vault_with(&store, Some(10_000), Some("ghr_old"));
        // fresh: no network
        assert_eq!(
            vault
                .access_token(&Mock::new(vec![]), &provider(), 1000)
                .unwrap()
                .as_deref(),
            Some("ghu_old")
        );
        // within the margin: refreshed, both stored again
        let t = Mock::new(vec![Ok(
            json!({"access_token": "ghu_new", "refresh_token": "ghr_new", "expires_in": 28800}),
        )]);
        assert_eq!(
            vault
                .access_token(&t, &provider(), 9_990)
                .unwrap()
                .as_deref(),
            Some("ghu_new")
        );
        let sent = &t.calls.borrow()[0].1;
        assert!(sent.contains(&("refresh_token".into(), "ghr_old".into())));
        assert!(sent.contains(&("grant_type".into(), "refresh_token".into())));
        let saved = vault.load().unwrap().unwrap();
        assert_eq!(saved.refresh_token.as_deref(), Some("ghr_new"));
        assert_eq!(saved.expires_at, Some(9_990 + 28800));
    }

    #[test]
    fn a_refused_refresh_or_a_missing_refresh_token_signs_out_locally() {
        let store = Checked(MemorySecrets::default());
        let vault = TokenVault::new(&store, "gh");
        vault_with(&store, Some(10), Some("ghr_dead"));
        let t = Mock::new(vec![Ok(json!({"error": "bad_refresh_token"}))]);
        assert_eq!(vault.access_token(&t, &provider(), 100).unwrap(), None);
        assert!(!vault.session().unwrap().connected);
        vault_with(&store, Some(10), None);
        assert_eq!(
            vault
                .access_token(&Mock::new(vec![]), &provider(), 100)
                .unwrap(),
            None
        );
        assert_eq!(store.get("gh.access").unwrap(), None);
    }

    #[test]
    fn a_network_error_while_refreshing_keeps_the_tokens() {
        let store = Checked(MemorySecrets::default());
        let vault = TokenVault::new(&store, "gh");
        vault_with(&store, Some(10), Some("ghr"));
        let t = Mock::new(vec![Err(TransportError("offline".into()))]);
        assert!(vault.access_token(&t, &provider(), 100).is_err());
        assert!(vault.session().unwrap().connected);
    }

    #[test]
    fn sign_out_removes_every_secret_and_twice_is_fine() {
        let store = Checked(MemorySecrets::default());
        vault_with(&store, Some(10), Some("r"));
        let vault = TokenVault::new(&store, "gh");
        vault.sign_out().unwrap();
        vault.sign_out().unwrap();
        for k in ["gh.access", "gh.refresh", "gh.expires"] {
            assert_eq!(store.get(k).unwrap(), None, "{k}");
        }
    }

    #[test]
    fn the_session_the_page_sees_holds_no_token() {
        let store = Checked(MemorySecrets::default());
        vault_with(&store, None, None);
        let s = serde_json::to_string(&TokenVault::new(&store, "gh").session().unwrap()).unwrap();
        assert_eq!(s, r#"{"connected":true}"#);
    }
}
