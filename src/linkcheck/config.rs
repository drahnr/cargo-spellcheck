//! Link check configuration.
use serde::{Deserialize, Serialize};

/// Parameters for wrapping doc comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkCheckConfig {
    /// Ignore private ip ranges.
    #[serde(default)]
    pub(crate) exclude_private_ips: bool,
}

impl Default for LinkCheckConfig {
    fn default() -> Self {
        Self {
            exclude_private_ips: true,
        }
    }
}
