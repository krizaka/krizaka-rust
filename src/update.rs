//! The update channel of a Mac app: `stable` by default, `beta` when the person (or the product's admin) chooses it.
//!
//! The channel is data: the product gives the feed address as a template with `{channel}` (for instance
//! `https://github.com/krizaka/verikora/releases/download/desktop-{channel}/latest.json`), the kit fills it.
//!
//! What the kit wires, and what it does not:
//! - it stores the channel on the Mac and gives the feed of that channel ([`feed_url`]: HTTPS, no credentials, names the
//!   channel);
//! - it downloads nothing and installs nothing. An app that ships updates registers Tauri's updater plugin itself, and
//!   that plugin checks each download against the `pubkey` of the app's `plugins.updater` configuration;
//! - so the kit refuses to start when the app gives a feed, or configures the updater, without a public key
//!   ([`require_pubkey`]): a feed address alone must never be able to install anything.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use url::Url;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Stable, Channel::Beta];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The text is not the name of a channel. The text is not repeated: it came from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownChannel;

impl UnknownChannel {
    /// The stable code the page reads.
    pub fn code(self) -> &'static str {
        "unknown_channel"
    }
}

impl fmt::Display for UnknownChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown update channel")
    }
}

impl std::error::Error for UnknownChannel {}

impl FromStr for Channel {
    type Err = UnknownChannel;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Channel::ALL
            .into_iter()
            .find(|c| c.as_str() == input.trim())
            .ok_or(UnknownChannel)
    }
}

/// Why a feed template cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedError {
    /// The template does not contain `{channel}`.
    MissingChannel,
    NotAUrl,
    NotHttps,
    Credentials,
}

impl FeedError {
    /// The stable code the page reads.
    pub fn code(self) -> &'static str {
        match self {
            FeedError::MissingChannel => "feed_missing_channel",
            FeedError::NotAUrl => "feed_not_a_url",
            FeedError::NotHttps => "feed_not_https",
            FeedError::Credentials => "feed_credentials",
        }
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FeedError::MissingChannel => "the update feed must name {channel}",
            FeedError::NotAUrl => "the update feed is not a valid URL",
            FeedError::NotHttps => "the update feed must be served over https",
            FeedError::Credentials => "no credentials in the update feed",
        })
    }
}

impl std::error::Error for FeedError {}

/// The app configures the updater (or gives a feed) without a public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingPubkey;

impl MissingPubkey {
    /// The stable code the page reads.
    pub fn code(self) -> &'static str {
        "updater_missing_pubkey"
    }
}

impl fmt::Display for MissingPubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the updater needs a pubkey in plugins.updater of tauri.conf.json")
    }
}

impl std::error::Error for MissingPubkey {}

/// The feed of `channel`: the template with `{channel}` filled in. The template must be an HTTPS address without
/// credentials and must name the channel, so a beta build never reads the stable feed by mistake.
pub fn feed_url(template: &str, channel: Channel) -> Result<Url, FeedError> {
    if !template.contains("{channel}") {
        return Err(FeedError::MissingChannel);
    }
    let url = Url::parse(&template.replace("{channel}", channel.as_str()))
        .map_err(|_| FeedError::NotAUrl)?;
    if url.scheme() != "https" {
        return Err(FeedError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FeedError::Credentials);
    }
    Ok(url)
}

/// The app's updater configuration (`plugins.updater` of `tauri.conf.json`, `None` when it has none) holds a public key.
///
/// Called before the kit offers a feed or when the app configures the updater: without a key, nothing would check what
/// the feed points to.
pub fn require_pubkey(updater: Option<&serde_json::Value>) -> Result<(), MissingPubkey> {
    let has_key = updater
        .and_then(|section| section.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|key| !key.trim().is_empty());
    if has_key {
        Ok(())
    } else {
        Err(MissingPubkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str =
        "https://github.com/krizaka/verikora/releases/download/desktop-{channel}/latest.json";

    #[test]
    fn stable_is_the_default_and_a_channel_reads_back_from_its_name() {
        assert_eq!(Channel::default(), Channel::Stable);
        for channel in Channel::ALL {
            assert_eq!(channel.as_str().parse::<Channel>().unwrap(), channel);
        }
        assert_eq!(" beta ".parse::<Channel>().unwrap(), Channel::Beta);
        assert!("nightly".parse::<Channel>().is_err());
        assert!("".parse::<Channel>().is_err());
        assert_eq!(serde_json::to_string(&Channel::Beta).unwrap(), "\"beta\"");
    }

    #[test]
    fn the_feed_names_the_channel() {
        assert_eq!(
            feed_url(FEED, Channel::Beta).unwrap().as_str(),
            "https://github.com/krizaka/verikora/releases/download/desktop-beta/latest.json"
        );
        assert!(feed_url(FEED, Channel::Stable)
            .unwrap()
            .as_str()
            .contains("desktop-stable"));
    }

    #[test]
    fn refuses_a_feed_that_is_not_https_names_no_channel_or_carries_credentials() {
        for bad in [
            "http://github.com/{channel}/latest.json",
            "https://github.com/latest.json",
            "https://user:pw@github.com/{channel}/latest.json",
            "file:///{channel}/latest.json",
            "{channel}",
        ] {
            assert!(feed_url(bad, Channel::Stable).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_updater_is_refused_without_a_public_key() {
        let missing = [
            None,
            Some(serde_json::json!({})),
            Some(serde_json::json!({ "endpoints": [FEED] })),
            Some(serde_json::json!({ "pubkey": "" })),
            Some(serde_json::json!({ "pubkey": "  \n" })),
            Some(serde_json::json!({ "pubkey": null })),
            Some(serde_json::json!({ "pubkey": 42 })),
        ];
        for updater in &missing {
            assert!(require_pubkey(updater.as_ref()).is_err(), "{updater:?}");
        }
        let configured =
            serde_json::json!({ "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk=" });
        assert_eq!(require_pubkey(Some(&configured)), Ok(()));
    }
}
