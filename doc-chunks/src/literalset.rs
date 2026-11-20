pub use super::{TrimmedLiteral, TrimmedLiteralDisplay};

use crate::{CheckableChunk, CommentVariant, Range};

use std::fmt;

#[cfg(test)]
mod tests;

/// An ordered set of concrete, source-mapped comment literals.
///
/// Provides means to render them as a code block
///
/// Selected documentation literals may have source gaps; developer comments remain adjacent.
#[derive(Clone, Default, Debug, Hash, PartialEq, Eq)]
pub struct LiteralSet {
    /// Concrete literals in document order, each retaining its source span.
    literals: Vec<TrimmedLiteral>,
    /// lines spanned (start, end) inclusive
    pub coverage: (usize, usize),
    /// Track what kind of comment the literals are
    variant: CommentVariant,
    /// Attribute syntax or conditional projections prevent whole-block edits.
    non_reflowable: bool,
    /// Example cfg assignment selecting this documentation variant.
    cfg_example: Option<String>,
}

impl LiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: TrimmedLiteral) -> Self {
        Self {
            coverage: (literal.span().start.line, literal.span().end.line),
            variant: literal.variant(),
            literals: vec![literal],
            non_reflowable: false,
            cfg_example: None,
        }
    }

    /// Build a complete documentation variant from known literals in document order.
    /// Selected literals need not be on adjacent source lines (inactive attributes leave gaps).
    pub(crate) fn from_doc_literals(literals: Vec<TrimmedLiteral>, non_reflowable: bool) -> Self {
        let first = literals
            .first()
            .expect("a documentation variant is nonempty");
        let last = literals.last().unwrap();
        Self {
            coverage: (first.span().start.line, last.span().end.line),
            variant: first.variant(),
            literals,
            non_reflowable,
            cfg_example: None,
        }
    }

    /// Attach an example cfg assignment for diagnostics.
    pub(crate) fn set_cfg_example(&mut self, example: String) {
        self.cfg_example = Some(example);
    }

    /// Add a literal to a literal set, if the previous lines literal already
    /// exists.
    ///
    /// Returns literal within the Err variant if not adjacent
    pub fn add_adjacent(&mut self, literal: TrimmedLiteral) -> Result<(), TrimmedLiteral> {
        if literal.variant().category() != self.variant.category() {
            log::debug!(
                "Adjacent literal is not the same comment variant: {:?} vs {:?}",
                literal.variant().category(),
                self.variant.category()
            );
            return Err(literal);
        }
        let previous_line = literal.span().end.line;
        if previous_line == self.coverage.1 + 1 {
            self.coverage.1 += 1;
            self.literals.push(literal);
            return Ok(());
        }

        let next_line = literal.span().start.line;
        if next_line + 1 == self.coverage.0 {
            self.literals.push(literal);
            self.coverage.1 -= 1;
            return Ok(());
        }

        Err(literal)
    }

    /// The set of trimmed literals that is covered.
    pub fn literals(&self) -> Vec<&TrimmedLiteral> {
        self.literals.iter().by_ref().collect()
    }

    /// The number of literals inside this set.
    pub fn len(&self) -> usize {
        self.literals.len()
    }

    /// Whether this set contains no literals.
    pub fn is_empty(&self) -> bool {
        self.literals.is_empty()
    }

    /// Convert to a checkable chunk.
    ///
    /// Creates the map from content ranges to source spans.
    pub fn into_chunk(self) -> crate::CheckableChunk {
        let preferred_line_delimiter = self
            .literals
            .iter()
            .find_map(TrimmedLiteral::preferred_line_delimiter);
        let n = self.len();
        let mut source_mapping = indexmap::IndexMap::with_capacity(n);
        let mut content = String::with_capacity(n * 120);
        if n > 0 {
            // cursor operates on characters
            let mut cursor = 0usize;
            // for use with `Range`
            let mut start; // inclusive
            let mut end; // exclusive
            let mut it = self.literals.iter();
            let mut next = it.next();
            while let Some(literal) = next {
                start = cursor;
                cursor += literal.len_in_chars();
                end = cursor;

                let span = literal.span();
                let range = Range { start, end };

                // TODO this does not hold anymore for `#[doc=foo!(..)]`.
                // TODO where the span is covering `foo!()`, but the
                // TODO rendered length is 0.
                if literal.variant() != CommentVariant::MacroDocEqMacro {
                    if let Some(span_len) = span.one_line_len() {
                        assert_eq!(range.len(), span_len);
                    }
                }
                // keep zero length values too, to guarantee continuity
                source_mapping.insert(range, span);
                content.push_str(literal.as_str());
                // the newline is _not_ covered by a span, after all it's inserted by us!
                next = it.next();
                if next.is_some() {
                    // for the last, skip the newline
                    content.push('\n');
                    cursor += 1;
                }
            }
        }
        // Selected literals can mix syntax; use the first one's rendering style.
        let variant = if let Some(literal) = self.literals.first() {
            literal.variant()
        } else {
            crate::CommentVariant::Unknown
        };
        let mut chunk = CheckableChunk::from_string(content, source_mapping, variant);
        // Synthetic separators must not override the literals' source style.
        chunk.set_preferred_line_delimiter(preferred_line_delimiter);
        chunk.set_cfg_example(self.cfg_example);
        if self.non_reflowable {
            chunk.disable_reflow();
        }
        chunk
    }
}

impl fmt::Display for LiteralSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.len();
        if n > 0 {
            for literal in self.literals.iter().take(n - 1) {
                writeln!(formatter, "{}", literal.as_str())?;
            }
            if let Some(literal) = self.literals.last() {
                write!(formatter, "{}", literal.as_str())?;
            }
        }
        Ok(())
    }
}
/// A debug helper to print concatenated length of all items.
#[macro_export]
macro_rules! chyrp_dbg {
    ($first:literal $(, $( $line:literal ),+ )? $(,)? $(@ $prefix:literal)? ) => {
        dbg!(concat!($first $( $(, "\n", $line )+ )?).len());
        dbg!(concat!($first $( $(, "\n", $line )+ )?));
    }
}

/// A helper macro creating valid doc string using the macro syntax
/// `#[doc=r#"..."#]`.
///
/// Example:
///
/// ```rust
/// # use doc_chunks::chyrp_up;
/// let x = chyrp_up!(["some", "thing"]);
/// let y = r##"#[doc=r#"some
/// thing"#]
/// struct ChyrpChyrp;"##;
///
/// assert_eq!(x,y);
/// ```
#[macro_export]
macro_rules! chyrp_up {
    ([ $( $line:literal ),+ $(,)? ] $(@ $prefix:literal)? ) => {
        chyrp_up!( $( $line ),+ $(@ $prefix)? )
    };
    ($first:literal $(, $( $line:literal ),+ )? $(,)? $(@ $prefix:literal)? ) => {
        concat!($( $prefix ,)? r##"#[doc=r#""##, $first $( $(, "\n", $line )+ )?, r##""#]"##, "\n", "struct ChyrpChyrp;")
    };
}

/// A helper macro creating valid doc string using the macro syntax
/// `/// ...`.
///
/// Example:
///
/// ```rust
/// # use doc_chunks::fluff_up;
/// let x = fluff_up!(["some", "thing"]);
/// let y = r#"/// some
/// /// thing
/// struct Fluff;"#;
///
/// assert_eq!(x,y);
/// ```
#[macro_export]
macro_rules! fluff_up {
    ([ $( $line:literal ),+ $(,)?] $( @ $prefix:literal)?) => {
        fluff_up!($( $line ),+ $(@ $prefix)?)
    };
    ($($line:literal ),+ $(,)? ) => {
        fluff_up!($( $line ),+ @ "")
    };
    ($($line:literal ),+ $(,)? @ $prefix:literal ) => {
        concat!("" $(, $prefix, "/// ", $line, "\n")+ , "struct Fluff;")
    };
}

pub mod testhelper {
    use super::*;
    use crate::testcase::annotated_literals;

    pub fn gen_literal_set(source: &str) -> LiteralSet {
        let literals = dbg!(annotated_literals(dbg!(source)));

        let mut iter = dbg!(literals).into_iter();
        let literal = iter
            .next()
            .expect("Must have at least one item in laterals");
        let mut cls = LiteralSet::from(literal);

        for literal in iter {
            assert!(cls.add_adjacent(literal).is_ok());
        }
        dbg!(cls)
    }
}
