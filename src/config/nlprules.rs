//! NlpRules checker configuration.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct LanguageToolConfig {
    pub url: url::Url,
}

impl LanguageToolConfig {
    pub fn url(&self) -> &url::Url {
        &self.url
    }
}
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct NlpRulesConfig {
    /// Location to use for an initial lookup
    /// of alternate tokenizer and rules data.
    pub override_rules: Option<PathBuf>,
    pub override_tokenizer: Option<PathBuf>,
}
