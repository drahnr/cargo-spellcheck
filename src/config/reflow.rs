//! Reflow configuration.
use serde::{Deserialize, Serialize};

/// Parameters for wrapping doc comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflowConfig {
    /// Hard limit for absolute length of lines.
    #[serde(default)]
    #[serde(alias = "max_line_width")]
    pub(crate) max_line_length: usize,
}

impl Default for ReflowConfig {
    fn default() -> Self {
        Self {
            max_line_length: 80,
        }
    }
}
