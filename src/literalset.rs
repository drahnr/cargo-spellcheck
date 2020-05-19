use crate::{Span,LineColumn};
use super::*;

use std::fs;

use indexmap::IndexMap;
use log::{debug, info, trace, warn};
use proc_macro2::{Spacing, TokenTree};

use std::path::{Path, PathBuf};





#[derive(Clone,Debug,Copy)]
/// A litteral with meta info where the first and list whitespace may be found.
pub struct AnnotatedLiteralRef<'l> {
    reference: &'l AnnotatedLiteral,
}

impl<'l> std::ops::Deref for AnnotatedLiteralRef<'l> {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.reference.literal
    }
}

impl<'l> From<&'l AnnotatedLiteral> for AnnotatedLiteralRef<'l> {
	fn from(anno: &'l AnnotatedLiteral) -> Self {
		Self {
			reference: anno,
		}
	}
}

impl<'l> AnnotatedLiteralRef<'l> {
	pub fn pre(&self) -> usize {
		self.reference.pre
	}
	pub fn post(&self) -> usize {
		self.reference.pre
	}
	pub fn as_str(&self) -> &str {
		self.reference.as_str()
	}
	pub fn len(&self) -> usize {
		self.reference.len
	}
}



#[derive(Clone,Debug)]
/// A litteral with meta info where the first and list whitespace may be found.
pub struct AnnotatedLiteral {
    /// The literal which this annotates to.
    pub literal: proc_macro2::Literal,
    /// the complete rendered string including post and pre.
    pub rendered: String,
    /// Whitespace prefix len + 1
    pub pre: usize,
    /// Whitespace postfix len + 1
    pub post: usize,
    /// Length without pre and post
    /// if all whitespace, this is zer0 and the sum of pre+post is 2x len
    pub len: usize,
}

impl From<proc_macro2::Literal> for AnnotatedLiteral {
    fn from(literal: proc_macro2::Literal) -> Self {
        let rendered = literal.to_string();
        let scrap = |c: &'_ char| -> bool { c.is_whitespace() };
        let pre = 1 + rendered.chars().take_while(scrap).count();
        let post = 1 + rendered.chars().rev().take_while(scrap).count();
        Self {
            len: if rendered.len() > pre + post {rendered.len() - pre - post} else { 0 },
            literal,
            rendered,
            pre,
            post,
        }
    }
}

impl std::ops::Deref for AnnotatedLiteral {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.literal
    }
}

impl AnnotatedLiteral {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre+self.len)]
    }
}





#[derive(Clone, Debug, Default)]
pub struct ConsecutiveLiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<AnnotatedLiteral>,
    /// lines spanned (start, end)
    pub coverage: (usize, usize),
}

impl ConsecutiveLiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: AnnotatedLiteral) -> Self {
        Self {
            coverage: (literal.span().start().line, literal.span().end().line),
            literals: vec![literal],
        }
    }

    /// Add a literal to a literal set, if the previous lines literal already exists.
    ///
    /// Returns literl within the Err variant if not adjacent
    pub fn add_adjacent(
        &mut self,
        literal: AnnotatedLiteral,
    ) -> Result<(), AnnotatedLiteral> {
        let previous_line = literal.span().end().line;
        if previous_line == self.coverage.0 + 1 {
            let _ = self.literals.insert(previous_line, literal);
            return Ok(());
        }

        let next_line = literal.span().start().line;
        if next_line + 1 == self.coverage.1 {
            let _ = self.literals.insert(next_line, literal);
            return Ok(());
        }

        return Err(literal);
    }

    fn extract<'a>(
        it: &mut impl Iterator<Item = &'a AnnotatedLiteral>,
        mut offset: usize,
    ) -> Option<(&'a AnnotatedLiteral, LineColumn, usize)> {
        for literal in it {
            let len = literal.to_string().len();
            if offset > len {
                offset -= len;
                continue;
            }
            return Some((
                literal,
                LineColumn {
                    line: literal.span().start().line,
                    column: literal.span().start().column + offset,
                },
                offset,
            ));
        }
        None
    }

    /// Convert a linear offset to a set of offsets with literal references and spans within that literal.
    pub fn linear_coverage_to_span<'a>(
        &'a self,
        offset: usize,
        length: usize,
    ) -> Option<(&'a AnnotatedLiteral, Span)> {
        let mut x = self.literals.iter();

        if let Some((start_literal, start, mut offset)) = dbg!(Self::extract(&mut x, offset)) {
            offset += length;
            if let Some((_end_literal, end, _offset)) = dbg!(Self::extract(&mut x, offset)) {
                // if start_literal.span() != end_literal.span() {
                //     warn!("Need multiline literal coverage support #TODO");
                // }
                let span = Span {
                    start,
                    end,
                };
                return Some((start_literal, span))
            }
        }
        None
    }

    pub fn literals<'x>(&'x self) -> Vec<&'x AnnotatedLiteral> {
        self.literals.iter().by_ref().collect()
    }
}

use std::fmt;

impl<'s> fmt::Display for ConsecutiveLiteralSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for literal in self.literals.iter() {
            literal.fmt(formatter)?;
            formatter.write_str("\n")?;
        }
        Ok(())
    }
}