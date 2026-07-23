//! Qualified names (prefix, local name, resolved namespace).

use std::fmt;
use std::sync::Arc;

/// Namespace URIs are stored shared because they repeat constantly within a
/// document.
pub type NsUri = Arc<str>;

/// Implicitly bound namespace of the `xml` prefix.
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";
/// Namespace for `xmlns` declaration attributes.
pub const XMLNS_NAMESPACE: &str = "http://www.w3.org/2000/xmlns/";

/// A qualified name as written in the document, together with the resolved
/// namespace (if any).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QName {
    pub prefix: Option<String>,
    pub local: String,
    pub namespace: Option<NsUri>,
}

impl QName {
    pub fn new(prefix: Option<&str>, local: &str, namespace: Option<NsUri>) -> Self {
        Self {
            prefix: prefix.map(str::to_owned),
            local: local.to_owned(),
            namespace,
        }
    }

    /// Splits a raw name into prefix and local name.
    /// `a:b` → (`a`, `b`); names without (or with an empty) prefix stay
    /// unsplit.
    pub fn split(raw: &str) -> (Option<&str>, &str) {
        match raw.split_once(':') {
            Some((prefix, local)) if !prefix.is_empty() && !local.is_empty() => {
                (Some(prefix), local)
            }
            _ => (None, raw),
        }
    }

    /// The name as written in the document (`prefix:local`).
    pub fn as_written(&self) -> String {
        match &self.prefix {
            Some(prefix) => format!("{prefix}:{}", self.local),
            None => self.local.clone(),
        }
    }

    /// Comparison against an expected namespace and local name.
    pub fn matches(&self, namespace: Option<&str>, local: &str) -> bool {
        self.local == local && self.namespace.as_deref() == namespace
    }
}

impl fmt::Display for QName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.prefix {
            Some(prefix) => write!(f, "{prefix}:{}", self.local),
            None => write!(f, "{}", self.local),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_names() {
        assert_eq!(QName::split("foo"), (None, "foo"));
        assert_eq!(QName::split("a:b"), (Some("a"), "b"));
        assert_eq!(QName::split(":b"), (None, ":b"));
        assert_eq!(QName::split("a:"), (None, "a:"));
    }

    #[test]
    fn display_and_matches() {
        let ns: NsUri = Arc::from("urn:example");
        let name = QName::new(Some("ex"), "item", Some(ns));
        assert_eq!(name.to_string(), "ex:item");
        assert!(name.matches(Some("urn:example"), "item"));
        assert!(!name.matches(None, "item"));
    }
}
