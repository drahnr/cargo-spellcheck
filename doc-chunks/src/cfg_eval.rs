//! Configuration expression evaluator for `cfg` and `cfg_attr` attributes.
//!
//! This module provides functionality to evaluate Rust's `cfg` expressions
//! based on target architecture, host architecture, and enabled features.

use crate::errors::*;
use proc_macro2::TokenTree;
use std::collections::HashSet;

/// Configuration context for evaluating `cfg` expressions.
#[derive(Debug, Clone, Default)]
pub struct CfgContext {
    /// Target triple (e.g., "x86_64-unknown-linux-gnu")
    pub target: Option<String>,
    /// Host triple (defaults to current host)
    pub host: Option<String>,
    /// Set of enabled features
    pub features: HashSet<String>,
    /// Whether to evaluate cfg expressions (if false, all cfg_attr docs are included)
    pub eval_cfg: bool,
}

impl CfgContext {
    /// Create a new CfgContext with the given parameters.
    pub fn new(
        target: Option<String>,
        host: Option<String>,
        features: Vec<String>,
        eval_cfg: bool,
    ) -> Self {
        Self {
            target,
            host,
            features: features.into_iter().collect(),
            eval_cfg,
        }
    }

    /// Evaluate a cfg expression given as a token stream.
    ///
    /// Returns `true` if the condition is satisfied, `false` otherwise.
    /// If `eval_cfg` is false, always returns `true`.
    pub fn evaluate(&self, tokens: proc_macro2::TokenStream) -> Result<bool> {
        if !self.eval_cfg {
            // When evaluation is disabled, include all cfg_attr docs
            return Ok(true);
        }

        let mut tokens = tokens.into_iter();
        if let Some(token) = tokens.next() {
            self.evaluate_token(token, &mut tokens)
        } else {
            // Empty condition is considered true
            Ok(true)
        }
    }

    fn evaluate_token(
        &self,
        token: TokenTree,
        remaining: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        match token {
            TokenTree::Ident(ident) => {
                let ident_str = ident.to_string();
                match ident_str.as_str() {
                    "feature" => self.evaluate_feature(remaining),
                    "not" => self.evaluate_not(remaining),
                    "all" => self.evaluate_all(remaining),
                    "any" => self.evaluate_any(remaining),
                    "target_arch" => self.evaluate_target_arch(remaining),
                    "target_os" => self.evaluate_target_os(remaining),
                    "target_family" => self.evaluate_target_family(remaining),
                    "target_env" => self.evaluate_target_env(remaining),
                    "target_endian" => self.evaluate_target_endian(remaining),
                    "target_pointer_width" => self.evaluate_target_pointer_width(remaining),
                    "target_vendor" => self.evaluate_target_vendor(remaining),
                    "unix" => Ok(self.is_unix_target()),
                    "windows" => Ok(self.is_windows_target()),
                    // Unknown predicates are considered false
                    _ => {
                        log::debug!("Unknown cfg predicate: {}", ident_str);
                        Ok(false)
                    }
                }
            }
            TokenTree::Group(group) => {
                // Evaluate the contents of the group
                self.evaluate(group.stream())
            }
            _ => {
                // Unexpected token type
                Err(Error::Span(format!(
                    "Unexpected token in cfg expression: {:?}",
                    token
                )))
            }
        }
    }

    fn evaluate_feature(&self, tokens: &mut proc_macro2::token_stream::IntoIter) -> Result<bool> {
        // Skip '='
        if let Some(TokenTree::Punct(punct)) = tokens.next() {
            if punct.as_char() != '=' {
                return Err(Error::Span("Expected '=' after 'feature'".to_string()));
            }
        } else {
            return Err(Error::Span("Expected '=' after 'feature'".to_string()));
        }

        // Get the feature name
        if let Some(TokenTree::Literal(lit)) = tokens.next() {
            let feature = lit.to_string().trim_matches('"').to_string();
            Ok(self.features.contains(&feature))
        } else {
            Err(Error::Span(
                "Expected feature name after 'feature ='".to_string(),
            ))
        }
    }

    fn evaluate_not(&self, tokens: &mut proc_macro2::token_stream::IntoIter) -> Result<bool> {
        // Get the group containing the negated expression
        if let Some(TokenTree::Group(group)) = tokens.next() {
            let result = self.evaluate(group.stream())?;
            Ok(!result)
        } else {
            Err(Error::Span("Expected group after 'not'".to_string()))
        }
    }

    fn evaluate_all(&self, tokens: &mut proc_macro2::token_stream::IntoIter) -> Result<bool> {
        // Get the group containing the expressions
        if let Some(TokenTree::Group(group)) = tokens.next() {
            let stream = group.stream();
            let conditions = self.parse_comma_separated(stream)?;

            // All conditions must be true
            for condition in conditions {
                if !self.evaluate(condition)? {
                    return Ok(false);
                }
            }
            Ok(true)
        } else {
            Err(Error::Span("Expected group after 'all'".to_string()))
        }
    }

    fn evaluate_any(&self, tokens: &mut proc_macro2::token_stream::IntoIter) -> Result<bool> {
        // Get the group containing the expressions
        if let Some(TokenTree::Group(group)) = tokens.next() {
            let stream = group.stream();
            let conditions = self.parse_comma_separated(stream)?;

            // At least one condition must be true
            for condition in conditions {
                if self.evaluate(condition)? {
                    return Ok(true);
                }
            }
            Ok(false)
        } else {
            Err(Error::Span("Expected group after 'any'".to_string()))
        }
    }

    fn parse_comma_separated(
        &self,
        stream: proc_macro2::TokenStream,
    ) -> Result<Vec<proc_macro2::TokenStream>> {
        let mut conditions = Vec::new();
        let mut current = proc_macro2::TokenStream::new();

        for token in stream {
            match &token {
                TokenTree::Punct(punct) if punct.as_char() == ',' => {
                    if !current.is_empty() {
                        conditions.push(current);
                        current = proc_macro2::TokenStream::new();
                    }
                }
                _ => {
                    current.extend(std::iter::once(token));
                }
            }
        }

        if !current.is_empty() {
            conditions.push(current);
        }

        Ok(conditions)
    }

    fn evaluate_target_arch(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let arch = self.get_string_value(tokens, "target_arch")?;
        Ok(self.target_matches_arch(&arch))
    }

    fn evaluate_target_os(&self, tokens: &mut proc_macro2::token_stream::IntoIter) -> Result<bool> {
        let os = self.get_string_value(tokens, "target_os")?;
        Ok(self.target_matches_os(&os))
    }

    fn evaluate_target_family(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let family = self.get_string_value(tokens, "target_family")?;
        Ok(self.target_matches_family(&family))
    }

    fn evaluate_target_env(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let env = self.get_string_value(tokens, "target_env")?;
        Ok(self.target_matches_env(&env))
    }

    fn evaluate_target_endian(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let endian = self.get_string_value(tokens, "target_endian")?;
        Ok(self.target_matches_endian(&endian))
    }

    fn evaluate_target_pointer_width(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let width = self.get_string_value(tokens, "target_pointer_width")?;
        Ok(self.target_matches_pointer_width(&width))
    }

    fn evaluate_target_vendor(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
    ) -> Result<bool> {
        let vendor = self.get_string_value(tokens, "target_vendor")?;
        Ok(self.target_matches_vendor(&vendor))
    }

    fn get_string_value(
        &self,
        tokens: &mut proc_macro2::token_stream::IntoIter,
        context: &str,
    ) -> Result<String> {
        // Skip '='
        if let Some(TokenTree::Punct(punct)) = tokens.next() {
            if punct.as_char() != '=' {
                return Err(Error::Span(format!("Expected '=' after '{}'", context)));
            }
        } else {
            return Err(Error::Span(format!("Expected '=' after '{}'", context)));
        }

        // Get the value
        if let Some(TokenTree::Literal(lit)) = tokens.next() {
            Ok(lit.to_string().trim_matches('"').to_string())
        } else {
            Err(Error::Span(format!(
                "Expected string value after '{} ='",
                context
            )))
        }
    }

    fn target_matches_arch(&self, arch: &str) -> bool {
        if let Some(ref target) = self.target {
            // Parse target triple: arch-vendor-os-env
            if let Some(target_arch) = target.split('-').next() {
                return target_arch == arch;
            }
        }
        false
    }

    fn target_matches_os(&self, os: &str) -> bool {
        if let Some(ref target) = self.target {
            let parts: Vec<_> = target.split('-').collect();
            if parts.len() >= 3 {
                // Handle special cases like "unknown-linux-gnu"
                if parts[2] == "linux" && os == "linux" {
                    return true;
                }
                if parts[2] == "windows" && os == "windows" {
                    return true;
                }
                if parts[2] == "darwin" && os == "macos" {
                    return true;
                }
                return parts[2] == os;
            }
        }
        false
    }

    fn target_matches_family(&self, family: &str) -> bool {
        match family {
            "unix" => self.is_unix_target(),
            "windows" => self.is_windows_target(),
            _ => false,
        }
    }

    fn target_matches_env(&self, env: &str) -> bool {
        if let Some(ref target) = self.target {
            let parts: Vec<_> = target.split('-').collect();
            if parts.len() >= 4 {
                return parts[3] == env;
            }
        }
        false
    }

    fn target_matches_endian(&self, endian: &str) -> bool {
        if let Some(ref target) = self.target {
            if let Some(arch) = target.split('-').next() {
                // Most architectures are little-endian
                let is_big_endian = matches!(
                    arch,
                    "mips" | "mips64" | "powerpc" | "powerpc64" | "s390x" | "sparc" | "sparc64"
                );
                return (endian == "big" && is_big_endian)
                    || (endian == "little" && !is_big_endian);
            }
        }
        false
    }

    fn target_matches_pointer_width(&self, width: &str) -> bool {
        if let Some(ref target) = self.target {
            if let Some(arch) = target.split('-').next() {
                let is_64_bit = arch.contains("64") || arch == "aarch64";
                return (width == "64" && is_64_bit) || (width == "32" && !is_64_bit);
            }
        }
        false
    }

    fn target_matches_vendor(&self, vendor: &str) -> bool {
        if let Some(ref target) = self.target {
            let parts: Vec<_> = target.split('-').collect();
            if parts.len() >= 2 {
                return parts[1] == vendor;
            }
        }
        false
    }

    fn is_unix_target(&self) -> bool {
        if let Some(ref target) = self.target {
            return target.contains("linux")
                || target.contains("darwin")
                || target.contains("macos")
                || target.contains("freebsd")
                || target.contains("netbsd")
                || target.contains("openbsd")
                || target.contains("android")
                || target.contains("ios");
        }
        false
    }

    fn is_windows_target(&self) -> bool {
        if let Some(ref target) = self.target {
            return target.contains("windows");
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_evaluation() {
        let ctx = CfgContext::new(None, None, vec!["foo".to_string(), "bar".to_string()], true);

        // Test feature = "foo" (should be true)
        let tokens = quote::quote! { feature = "foo" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        // Test feature = "baz" (should be false)
        let tokens = quote::quote! { feature = "baz" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);
    }

    #[test]
    fn test_not_evaluation() {
        let ctx = CfgContext::new(None, None, vec!["foo".to_string()], true);

        // Test not(feature = "foo") (should be false)
        let tokens = quote::quote! { not(feature = "foo") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);

        // Test not(feature = "bar") (should be true)
        let tokens = quote::quote! { not(feature = "bar") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);
    }

    #[test]
    fn test_all_evaluation() {
        let ctx = CfgContext::new(None, None, vec!["foo".to_string(), "bar".to_string()], true);

        // Test all(feature = "foo", feature = "bar") (should be true)
        let tokens = quote::quote! { all(feature = "foo", feature = "bar") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        // Test all(feature = "foo", feature = "baz") (should be false)
        let tokens = quote::quote! { all(feature = "foo", feature = "baz") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);
    }

    #[test]
    fn test_any_evaluation() {
        let ctx = CfgContext::new(None, None, vec!["foo".to_string()], true);

        // Test any(feature = "foo", feature = "bar") (should be true)
        let tokens = quote::quote! { any(feature = "foo", feature = "bar") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        // Test any(feature = "bar", feature = "baz") (should be false)
        let tokens = quote::quote! { any(feature = "bar", feature = "baz") };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);
    }

    #[test]
    fn test_target_arch_evaluation() {
        let ctx = CfgContext::new(
            Some("x86_64-unknown-linux-gnu".to_string()),
            None,
            vec![],
            true,
        );

        // Test target_arch = "x86_64" (should be true)
        let tokens = quote::quote! { target_arch = "x86_64" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        // Test target_arch = "aarch64" (should be false)
        let tokens = quote::quote! { target_arch = "aarch64" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);
    }

    #[test]
    fn test_target_os_evaluation() {
        let ctx = CfgContext::new(
            Some("x86_64-unknown-linux-gnu".to_string()),
            None,
            vec![],
            true,
        );

        // Test target_os = "linux" (should be true)
        let tokens = quote::quote! { target_os = "linux" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        // Test target_os = "windows" (should be false)
        let tokens = quote::quote! { target_os = "windows" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), false);
    }

    #[test]
    fn test_unix_windows_evaluation() {
        let linux_ctx = CfgContext::new(
            Some("x86_64-unknown-linux-gnu".to_string()),
            None,
            vec![],
            true,
        );

        // Test unix on Linux target (should be true)
        let tokens = quote::quote! { unix };
        assert_eq!(linux_ctx.evaluate(tokens).unwrap(), true);

        // Test windows on Linux target (should be false)
        let tokens = quote::quote! { windows };
        assert_eq!(linux_ctx.evaluate(tokens).unwrap(), false);

        let windows_ctx = CfgContext::new(
            Some("x86_64-pc-windows-msvc".to_string()),
            None,
            vec![],
            true,
        );

        // Test unix on Windows target (should be false)
        let tokens = quote::quote! { unix };
        assert_eq!(windows_ctx.evaluate(tokens).unwrap(), false);

        // Test windows on Windows target (should be true)
        let tokens = quote::quote! { windows };
        assert_eq!(windows_ctx.evaluate(tokens).unwrap(), true);
    }

    #[test]
    fn test_eval_cfg_disabled() {
        let ctx = CfgContext::new(
            None,
            None,
            vec![],
            false, // eval_cfg is disabled
        );

        // All conditions should return true when eval_cfg is disabled
        let tokens = quote::quote! { feature = "nonexistent" };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);

        let tokens = quote::quote! { not(unix) };
        assert_eq!(ctx.evaluate(tokens).unwrap(), true);
    }
}
