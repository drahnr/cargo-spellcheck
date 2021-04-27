//! Reflow configuration.
use serde::{Deserialize, Serialize};

/// Parameters for wrapping doc comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkCheckConfig {
    /// Do not check the existance of urls that point
    /// to sub urls of these domains.
    #[serde(default)]
    #[serde(alias = "basecamp")]
    pub(crate) basecamp: Vec<url::Url>,

    /// Avoid checking private IP urls
    pub(crate) exclude_private_ips: bool,
}

impl Default for LinkCheckConfig {
    fn default() -> Self {
        Self {
            basecamp: Vec::new(),
            exclude_private_ips: true,
        }
    }
}
