//! NlpRules checker configuration.
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

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
