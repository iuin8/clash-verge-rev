//! Stable subscription identity and display-name derivation.

use sha2::{Digest as _, Sha256};

use crate::core::openp2p::portable::OpenP2PImportSourceKind;

pub(crate) fn stable_id(
    kind: OpenP2PImportSourceKind,
    source_url: Option<&str>,
    name: &str,
    text: &str,
) -> std::string::String {
    let material = match kind {
        OpenP2PImportSourceKind::Url => format!("url\n{}\n", canonical_url(source_url.unwrap_or_default())),
        OpenP2PImportSourceKind::Local => format!("local\n{name}\n{text}"),
    };
    let digest = Sha256::digest(material.as_bytes());
    digest.iter().take(8).map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn canonical_url(raw: &str) -> std::string::String {
    reqwest::Url::parse(raw.trim()).map_or_else(|_| raw.trim().to_owned(), |url| url.to_string())
}

pub(crate) fn derive_name(kind: OpenP2PImportSourceKind, source_url: Option<&str>, text: &str) -> std::string::String {
    let document_name = document_display_name(text);
    if !document_name.is_empty() {
        return normalize_name(document_name);
    }
    let raw = match kind {
        // The URL path may carry a capability token; never derive a name from it.
        OpenP2PImportSourceKind::Url => source_url
            .and_then(|url| reqwest::Url::parse(url.trim()).ok())
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default(),
        OpenP2PImportSourceKind::Local => std::string::String::new(),
    };
    normalize_name(raw)
}

fn document_display_name(text: &str) -> std::string::String {
    let Ok(value) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text) else {
        return std::string::String::new();
    };
    let Some(document) = value.as_mapping() else {
        return std::string::String::new();
    };
    let Some(block) = document.get("x-openp2p").and_then(serde_yaml_ng::Value::as_mapping) else {
        return std::string::String::new();
    };
    if let Some(name) = block
        .get("name")
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    let Some(account) = block
        .get("accounts")
        .and_then(serde_yaml_ng::Value::as_sequence)
        .and_then(|accounts| accounts.first())
        .and_then(serde_yaml_ng::Value::as_mapping)
    else {
        return std::string::String::new();
    };
    if let Some(label) = account
        .get("label")
        .and_then(serde_yaml_ng::Value::as_str)
        .map(str::trim)
        .filter(|label| !label.is_empty())
    {
        return format!("OpenP2P {label}");
    }
    account
        .get("id")
        .and_then(serde_yaml_ng::Value::as_str)
        .map_or_else(std::string::String::new, |id| format!("OpenP2P {id}"))
}

fn normalize_name(raw: std::string::String) -> std::string::String {
    let cleaned = raw
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<std::string::String>();
    let truncated = cleaned.trim().chars().take(64).collect::<std::string::String>();
    if truncated.is_empty() {
        "OpenP2P subscription".into()
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_document_uses_its_openp2p_name() {
        let text = "x-openp2p:\n  version: 1\n  name: Home Lab\n  accounts: []\n";
        assert_eq!(
            derive_name(
                OpenP2PImportSourceKind::Url,
                Some("https://example.com/secret-token.yaml"),
                text
            ),
            "Home Lab"
        );
    }

    #[test]
    fn a_url_without_a_name_uses_only_the_host() {
        let text = "x-openp2p:\n  version: 1\n  accounts: []\n";
        assert_eq!(
            derive_name(
                OpenP2PImportSourceKind::Url,
                Some("https://example.com/secret-token.yaml"),
                text
            ),
            "example.com"
        );
    }
}
