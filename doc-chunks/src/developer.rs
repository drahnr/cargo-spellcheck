//! Extract non-documentation comments as concrete literals and source-adjacent literal sets.
//!
//! This path uses syntax tokens to preserve source text and positions. Documentation
//! attributes, cfg projection, and unexpanded macros belong to the documentation path.

use std::fmt;
use std::fmt::{Display, Formatter};

use ra_ap_syntax::{ast, AstToken};

use regex::Regex;

use super::*;

#[cfg(test)]
mod tests;

/// Prefix string for a developer block comment
const BLOCK_COMMENT_PREFIX: &str = "/*";

/// Prefix string for a developer line comment
const LINE_COMMENT_PREFIX: &str = "//";

/// Prefix string for any other token type (i.e. we don't care)
const OTHER_PREFIX: &str = "";

/// Postfix string for a developer block comment
const BLOCK_COMMENT_POSTFIX: &str = "*/";

/// Postfix string for a developer line comment
const LINE_COMMENT_POSTFIX: &str = "";

/// Postfix string for any other token type (i.e. we don't care)
const OTHER_POSTFIX: &str = "";

lazy_static::lazy_static! {
  static ref BLOCK_COMMENT: Regex = Regex::new(r"^/\*(?s)(?P<content>.*)\*/$")
      .expect("Failed to create regular expression to identify (closed) developer block comments. \
          Please check this regex!");
  static ref LINE_COMMENT: Regex = Regex::new(r"^//([^[/|!]].*)?$")
      .expect("Failed to create regular expression to identify developer line comments. \
          Please check this regex!");
}

/// A string token from a source string with the location at which it occurs in
/// the source string as line on which it occurs (1 indexed) and the column of
/// its first character (0 indexed)
#[derive(Debug)]
struct TokenWithLineColumn {
    /// The full contents of this token, including pre/post characters (like
    /// '//')
    content: String,
    /// The first line on which the token appears in the source file (1 indexed)
    line: usize,
    /// The column where the first character of this token appears in the source
    /// file (0 indexed)
    column: usize,
}

/// Is a token of type (developer) block comment, (developer) line comment or
/// something else
#[derive(Debug, Eq, PartialEq)]
enum TokenType {
    BlockComment,
    LineComment,
    Other,
}

impl Display for TokenType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let kind = match self {
            TokenType::BlockComment => "developer block comment",
            TokenType::LineComment => "developer line comment",
            TokenType::Other => "not a developer comment",
        };
        write!(f, "{kind}")
    }
}

impl TokenType {
    /// The prefix string for this type of token
    fn pre(&self) -> &str {
        match self {
            TokenType::BlockComment => BLOCK_COMMENT_PREFIX,
            TokenType::LineComment => LINE_COMMENT_PREFIX,
            TokenType::Other => OTHER_PREFIX,
        }
    }
    /// The postfix string for this type of token
    fn post(&self) -> &str {
        match self {
            TokenType::BlockComment => BLOCK_COMMENT_POSTFIX,
            TokenType::LineComment => LINE_COMMENT_POSTFIX,
            TokenType::Other => OTHER_POSTFIX,
        }
    }
    /// The length of the prefix for the token in characters
    fn pre_in_chars(&self) -> usize {
        self.pre().chars().count()
    }
    /// The length of the postfix for the token in characters
    fn post_in_chars(&self) -> usize {
        self.post().chars().count()
    }
}

/// A token from a source string with its variant (`TokenType`) and the line and
/// column on which it occurs according to the description for
/// `TokenWithLineColumn`
#[derive(Debug)]
struct TokenWithType {
    /// Is the token a block developer comment, line developer comment or
    /// something else
    kind: TokenType,
    /// The full contents of this token, including pre/post characters (like
    /// '//')
    pub content: String,
    /// The first line on which the token appears in the source file (1 indexed)
    /// pub line: `usize`,
    pub line: usize,
    /// The column where the first character of this token appears in the source
    /// file (0 indexed)
    pub column: usize,
}

impl TokenWithType {
    /// Convert a `TokenWithLineColumn` to a `TokenWithType`. The kind is worked
    /// out from the content by checking against the developer block comment &
    /// line comment regexps.
    fn from(token: TokenWithLineColumn) -> Self {
        let kind = if BLOCK_COMMENT.is_match(&token.content) {
            TokenType::BlockComment
        } else if LINE_COMMENT.is_match(&token.content) {
            TokenType::LineComment
        } else {
            TokenType::Other
        };
        Self {
            kind,
            content: token.content,
            line: token.line,
            column: token.column,
        }
    }
}

/// A convenience method that runs the complete 'pipeline' from string `source`
/// file to all `LiteralSet`s that can be created from developer comments in the
/// source
pub fn extract_developer_comments(source: &str) -> Vec<LiteralSet> {
    let tokens = source_to_iter(source).collect::<Vec<_>>();

    construct_literal_sets(tokens)
}

/// Creates a series of `TokenWithType`s from a source string
fn source_to_iter(source: &str) -> impl Iterator<Item = TokenWithType> + '_ {
    // TODO: handle source
    let parse = ast::SourceFile::parse(source, ra_ap_syntax::Edition::Edition2021);
    let node = parse.syntax_node();
    node.descendants_with_tokens()
        .filter_map(|nort| {
            nort.into_token()
                .and_then(ast::Comment::cast)
                .filter(|comment| !comment.is_doc())
            // for now until it's clear whether #[doc=foo!()]
            // is possible with `ra_ap_syntax`
        })
        .map(move |comment| {
            let location = usize::from(comment.syntax().text_range().start());
            TokenWithType::from(TokenWithLineColumn {
                content: comment.text().to_owned(),
                line: count_lines(&source[..location]),
                column: calculate_column(&source[..location]),
            })
        })
}

/// Given a string, calculates the 1 indexed line number of the line on which
/// the final character of the string appears
fn count_lines(fragment: &str) -> usize {
    fragment.chars().filter(|c| *c == '\n').count() + 1
}

/// Given a string, calculates the 0 indexed column number of the character
/// *just after* the final character in the string
fn calculate_column(fragment: &str) -> usize {
    match fragment.rfind('\n') {
        Some(p) => fragment.chars().count() - fragment[..p].chars().count() - 1,
        None => fragment.chars().count(),
    }
}

/// Attempts to create a `LiteralSet` from a token assuming it is block comment.
/// Returns `None` if the token kind is not `TokenKind::BlockComment`, if the
/// token content does not match the block comment regex, or if any line cannot
/// be added by `LiteralSet::add_adjacent`
fn literal_set_from_block_comment(
    token: &TokenWithType,
) -> std::result::Result<LiteralSet, String> {
    let number_of_lines = token.content.split("\n").count();
    let mut lines = token.content.split("\n");
    if number_of_lines == 1 {
        let literal = match TrimmedLiteral::from(
        CommentVariant::SlashStar, &token.content, token.kind.pre_in_chars(),
        token.kind.post_in_chars(), token.line, token.column) {
      Err(s) => return Err(format!(
          "Failed to create literal from single line block comment, content \"{}\" - caused by \"{}\"",
          token.content, s)),
      Ok(l) => l
    };
        Ok(LiteralSet::from(literal))
    } else {
        let next_line = match lines.next() {
            None => {
                return Err(format!(
                    "BUG! Expected block comment \"{}\" to have at least two lines",
                    token.content
                ))
            }
            Some(l) => l,
        };
        let literal = match TrimmedLiteral::from(
            CommentVariant::SlashStar,
            next_line,
            token.kind.pre_in_chars(),
            0,
            token.line,
            token.column,
        ) {
            Err(s) => {
                return Err(format!(
                    "Failed to create literal from block comment with content \"{next_line}\" due to error \"{s}\"",
                ))
            }
            Ok(l) => l,
        };
        let mut literal_set = LiteralSet::from(literal);
        let mut line_number = token.line;
        for next_line in lines {
            line_number += 1;
            let post = if next_line.ends_with(BLOCK_COMMENT_POSTFIX) {
                TokenType::BlockComment.post_in_chars()
            } else {
                0
            };
            let literal = match TrimmedLiteral::from(
                CommentVariant::SlashStar,
                next_line,
                0,
                post,
                line_number,
                0,
            ) {
                Err(s) => {
                    return Err(format!(
                    "Failed to create literal from content \"{next_line}\" due to error \"{s}\"",
                ))
                }
                Ok(l) => l,
            };
            match literal_set.add_adjacent(literal) {
                Ok(_) => (),
                Err(_) => {
                    return Err(format!(
                        "Failed to add line with content {next_line} to literal set",
                    ))
                }
            }
        }
        Ok(literal_set)
    }
}

/// Attempt to create a literal from a developer line comment token. Returns
/// `None` if the token's kind is not `TokenType::LineComment` or if the call to
/// `TrimmedLiteral::from` fails.
fn literal_from_line_comment(token: &TokenWithType) -> std::result::Result<TrimmedLiteral, String> {
    match token.kind {
        TokenType::LineComment => TrimmedLiteral::from(
            CommentVariant::DoubleSlash,
            &token.content,
            token.kind.pre_in_chars(),
            token.kind.post_in_chars(),
            token.line,
            token.column,
        ),
        _ => Err(format!(
            "Expected a token of type {}, got {}",
            TokenType::LineComment,
            token.kind
        )),
    }
}

/// Converts a vector of tokens into a vector of `LiteralSet`s based on the
/// developer line comments in the input, ignoring all other tokens in the
/// input.
fn construct_literal_sets(tokens: impl IntoIterator<Item = TokenWithType>) -> Vec<LiteralSet> {
    let mut sets = vec![];
    'loopy: for token in tokens {
        let res = match token.kind {
            TokenType::LineComment => literal_from_line_comment(&token),
            TokenType::BlockComment => {
                if let Ok(set) = literal_set_from_block_comment(&token) {
                    sets.push(set)
                }
                continue 'loopy;
            }
            _ => continue 'loopy,
        };
        let literal = match res {
            Err(err) => {
                log::trace!(
                    "Failed to create literal from comment with content \"{}\" due to \"{}\"",
                    token.content,
                    err
                );
                continue 'loopy;
            }
            Ok(l) => l,
        };
        match sets.pop() {
            None => sets.push(LiteralSet::from(literal)),
            Some(mut s) => match s.add_adjacent(literal) {
                Err(literal) => {
                    sets.push(s);
                    sets.push(LiteralSet::from(literal))
                }
                Ok(_) => sets.push(s),
            },
        }
    }
    sets
}
