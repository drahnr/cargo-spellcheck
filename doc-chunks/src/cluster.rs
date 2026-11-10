//! Cluster `proc_macro2::Literal`s into `LiteralSets`

use proc_macro2::Delimiter;
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{parenthesized, Ident, LitStr, Macro, Token};

use super::cfg::Predicate;
use super::{LiteralSet, TokenTree, TrimmedLiteral};
use crate::developer::extract_developer_comments;
use crate::errors::*;
use crate::Span;

enum DocComment {
    LitStr(LitStr),
    Macro(Macro),
}

impl DocComment {
    fn into_literal(self, source: &str) -> Result<TrimmedLiteral> {
        match self {
            Self::LitStr(literal) => TrimmedLiteral::load_from(source, Span::from(literal.span())),
            Self::Macro(mac) => Ok(TrimmedLiteral::new_empty(
                source,
                Span::from(mac.span()),
                crate::CommentVariant::MacroDocEqMacro,
            )),
        }
    }
}

impl syn::parse::Parse for DocComment {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        if name.unraw() != "doc" {
            return Err(syn::Error::new_spanned(name, "expected a doc attribute"));
        }
        input.parse::<Token![=]>()?;
        if input.peek(LitStr) {
            input.parse().map(Self::LitStr)
        } else {
            input.parse().map(Self::Macro)
        }
    }
}

/// Only these two attribute forms contribute documentation. Other attribute
/// payloads are opaque, even if they contain tokens resembling `doc = ...`.
enum DocAttribute {
    Doc(DocComment),
    Conditional(Predicate, Vec<DocAttribute>),
    Other,
}

impl syn::parse::Parse for DocAttribute {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input
            .fork()
            .parse::<Ident>()
            .ok()
            .map(|name| name.unraw().to_string());
        if name.as_deref() == Some("doc") && input.peek2(Token![=]) {
            return input.parse().map(Self::Doc);
        }
        if name.as_deref() == Some("cfg_attr") && input.peek2(syn::token::Paren) {
            input.parse::<Ident>()?;
            let content;
            parenthesized!(content in input);
            let predicate = content.parse::<Predicate>()?;
            content.parse::<Token![,]>()?;
            let attributes = Punctuated::<DocAttribute, Token![,]>::parse_terminated(&content)?;
            return Ok(Self::Conditional(
                predicate,
                attributes.into_iter().collect(),
            ));
        }
        // A token group is consumed whole, so commas inside an unrelated
        // attribute's arguments cannot be mistaken for cfg_attr separators.
        while !input.is_empty() && !input.peek(Token![,]) {
            input.parse::<TokenTree>()?;
        }
        Ok(Self::Other)
    }
}

#[derive(Default)]
struct DocRun {
    docs: Vec<(Predicate, TrimmedLiteral)>,
    non_reflowable: bool,
    intervening_attribute: bool,
    incomplete: bool,
}

impl DocRun {
    fn append(&mut self, source: &str, attribute: DocAttribute, guard: Predicate) -> Result<()> {
        match attribute {
            DocAttribute::Doc(comment) => {
                let literal = comment.into_literal(source)?;
                if let Some((_, previous)) = self.docs.last() {
                    // Structural adjacency can cross syntax that reflow must
                    // never replace (or put several attributes on one line).
                    self.non_reflowable |= self.intervening_attribute
                        || literal.span().start.line != previous.span().end.line + 1
                        || literal.variant() != previous.variant();
                }
                if let crate::CommentVariant::MacroDocEqStr(prefix, _) = literal.variant() {
                    // Reflow repeats this prefix. It must not contain another
                    // item or be a fragment of a multiline attribute wrapper.
                    let prefix: String = prefix.chars().filter(|c| !c.is_whitespace()).collect();
                    self.non_reflowable |= !matches!(
                        prefix.as_str(),
                        "#[doc=" | "#![doc=" | "#[r#doc=" | "#![r#doc="
                    );
                }
                self.intervening_attribute = false;
                self.docs.push((guard, literal));
            }
            DocAttribute::Conditional(predicate, attributes) => {
                let previous_len = self.docs.len();
                let guard = Predicate::All(vec![guard, predicate]);
                for attribute in attributes {
                    self.append(source, attribute, guard.clone())?;
                }
                self.non_reflowable |= self.docs.len() != previous_len;
                self.intervening_attribute = true;
            }
            DocAttribute::Other => self.intervening_attribute = true,
        }
        Ok(())
    }

    fn finish(self, sets: &mut Vec<LiteralSet>) {
        if self.docs.is_empty() || self.incomplete {
            return;
        }
        let mut atoms = Vec::new();
        for (predicate, _) in &self.docs {
            predicate.collect_atoms(&mut atoms);
        }
        // Enumeration is local to an attribute run, never the whole file.
        // Bound the exponential work; flattening or arbitrarily choosing a
        // branch on overflow would produce misleading Markdown and findings.
        const MAX_ATOMS: usize = 10;
        if atoms.len() > MAX_ATOMS {
            log::warn!(
                "Skipping documentation at line {}: {} independent cfg predicates exceed the limit of {}",
                self.docs[0].1.span().start.line,
                atoms.len(),
                MAX_ATOMS,
            );
            return;
        }
        let mut seen = std::collections::HashSet::new();
        for assignment in 0..(1 << atoms.len()) {
            let selected: Vec<_> = self
                .docs
                .iter()
                .enumerate()
                .filter(|(_, (predicate, _))| predicate.matches(&atoms, assignment))
                .map(|(index, _)| index)
                .collect();
            // Deduplicate literal identities, not text: equal strings in
            // different branches still need corrections at both locations.
            if selected.is_empty() || !seen.insert(selected.clone()) {
                continue;
            }
            let literals = selected
                .into_iter()
                .map(|index| self.docs[index].1.clone())
                .collect();
            sets.push(LiteralSet::from_docs(literals, self.non_reflowable));
        }
    }
}

/// Cluster comments together, such they appear as continuous text blocks.
#[derive(Debug)]
pub struct Clusters {
    pub(crate) set: Vec<LiteralSet>,
}

impl Clusters {
    /// Read actual attributes, keeping each item's documentation together even
    /// across inactive or unrelated attributes. Ordinary tokens and scope or
    /// inner/outer attribute boundaries terminate the current documentation run.
    pub fn parse_token_tree(
        &mut self,
        source: &str,
        stream: proc_macro2::TokenStream,
    ) -> Result<()> {
        let mut iter = stream.into_iter().peekable();
        let mut run = DocRun::default();
        let mut inner_style = None;
        while let Some(tree) = iter.next() {
            if matches!(&tree, TokenTree::Punct(punct) if punct.as_char() == '#') {
                let inner = iter
                    .next_if(
                        |tree| matches!(tree, TokenTree::Punct(punct) if punct.as_char() == '!'),
                    )
                    .is_some();
                if let Some(TokenTree::Group(group)) = iter.next_if(
                    |tree| matches!(tree, TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket),
                ) {
                    if inner_style.is_some() && inner_style != Some(inner) {
                        std::mem::take(&mut run).finish(&mut self.set);
                    }
                    inner_style = Some(inner);
                    match syn::parse2::<DocAttribute>(group.stream()) {
                        Ok(attribute) => run.append(source, attribute, Predicate::always())?,
                        Err(error) => {
                            // Do not descend into an invalid/unsupported attribute
                            // and accidentally interpret its payload as docs.
                            log::warn!("Skipping documentation block with an unsupported attribute: {}", error);
                            // An omitted fragment could be a code fence. Do not
                            // spellcheck an incomplete Markdown projection.
                            run.incomplete = true;
                        }
                    }
                    continue;
                }
            }
            std::mem::take(&mut run).finish(&mut self.set);
            inner_style = None;
            if let TokenTree::Group(group) = tree {
                self.parse_token_tree(source, group.stream())?;
            }
        }
        run.finish(&mut self.set);
        Ok(())
    }

    /// From the given source text, extracts developer comments to `LiteralSet`s
    /// and adds them to this `Clusters`
    fn parse_developer_comments(&mut self, source: &str) {
        let developer_comments = extract_developer_comments(source);
        self.set.extend(developer_comments);
    }

    /// Sort the `LiteralSet`s in this `Cluster` by start line descending, to
    /// ensure that the comments higher up in the source file appear first to
    /// the user
    fn ensure_sorted(&mut self) {
        self.set.sort_by_key(|ls| ls.coverage);
    }

    /// Load clusters from a `&str`. Optionally loads developer comments as
    /// well.
    pub fn load_from_str(source: &str, doc_comments: bool, dev_comments: bool) -> Result<Self> {
        let mut chunk = Self {
            set: Vec::with_capacity(64),
        };
        if doc_comments {
            let stream =
                syn::parse_str::<proc_macro2::TokenStream>(source).map_err(Error::ParserFailure)?;
            chunk.parse_token_tree(source, stream)?;
        }
        if dev_comments {
            chunk.parse_developer_comments(source);
        }
        chunk.ensure_sorted();
        Ok(chunk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comment_parse() {
        let _ = syn::parse_str::<DocComment>(r########"doc=foo!(bar!(xxx))"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc="s""########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r#"s"#"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r##"s"##"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r###"s"###"########).unwrap();
        let _ = syn::parse_str::<DocComment>(r########"doc=r####"s"####"########).unwrap();
    }

    #[test]
    fn load_flags_keep_doc_and_developer_comments_independent() {
        let source = "/// Documentation\nstruct X;\n// Developer comment\n";
        for (docs, devs, expected) in [
            (false, false, 0),
            (true, false, 1),
            (false, true, 1),
            (true, true, 2),
        ] {
            let clusters = Clusters::load_from_str(source, docs, devs).unwrap();
            assert_eq!(clusters.set.len(), expected);
        }
    }

    #[test]
    fn conditional_run_keeps_repeated_predicates_correlated() {
        let source = r#"
#[cfg_attr(feature = "a", doc = "First")]
#[cfg_attr(feature = "a", doc = "Second")]
#[cfg_attr(not(feature = "a"), doc = "Alternative")]
struct X;
"#;
        let clusters = Clusters::load_from_str(source, true, false).unwrap();
        let mut lengths: Vec<_> = clusters.set.iter().map(LiteralSet::len).collect();
        lengths.sort_unstable();
        assert_eq!(lengths, vec![1, 2]);
    }

    #[test]
    fn conditional_run_atom_limit_remains_ten() {
        for (count, expected) in [(10, 1023), (11, 0)] {
            let mut source = String::new();
            for index in 0..count {
                source.push_str(&format!("#[cfg_attr(flag{index}, doc = \"Text\")]\n"));
            }
            source.push_str("struct X;\n");
            let clusters = Clusters::load_from_str(&source, true, false).unwrap();
            assert_eq!(clusters.set.len(), expected);
        }
    }

    #[test]
    fn create_cluster() {
        static CONTENT: &str = r#####"
mod mm_mm {

/// A
#[doc=foo!(B)]
/// C
#[doc=r##"D"##]
struct X;

}
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }

    #[test]
    fn space_in_code_block_does_not_break_cluster() {
        static CONTENT: &str = r#####"
// ```c
// hugloboi
//
// fucksteufelswuid
// ```
struct DefinitelyNotZ;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }

    #[test]
    fn polite() {
        static CONTENT: &str = r#####"
// Hello Sir
//
// How are you doing today?
struct VeryWellThanks;
"#####;
        let clusters = Clusters::load_from_str(CONTENT, true, true).unwrap();
        assert_eq!(clusters.set.len(), 1);
        dbg!(&clusters.set[0]);
    }
}
