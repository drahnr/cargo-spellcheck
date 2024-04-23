use std::fmt;
use std::fmt::{Display, Formatter};

use ra_ap_syntax::{ast, AstToken};

use regex::Regex;

use super::*;

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
    let comments = construct_literal_sets(tokens);
    comments
}

/// Creates a series of `TokenWithType`s from a source string
fn source_to_iter<'a>(source: &'a str) -> impl Iterator<Item = TokenWithType> + 'a {
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
    fragment.chars().into_iter().filter(|c| *c == '\n').count() + 1
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
        while let Some(next_line) = lines.next() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;

    #[test]
    fn test_count_lines_correctly_counts_lines() {
        // Note: lines are 1 indexed
        assert_eq!(count_lines(""), 1);
        assert_eq!(count_lines("test"), 1);
        assert_eq!(count_lines("test\ntest"), 2);
        assert_eq!(count_lines("test\ntest\n something else \n"), 4);
        assert_eq!(count_lines("\n test\ntest\n something else \n"), 5);
    }

    #[test]
    fn test_calculate_column_correctly_calculates_final_column_of_last_line() {
        // Note: next column after last, in chars, zero indexed
        assert_eq!(calculate_column(""), 0);
        assert_eq!(calculate_column("test中"), 5);
        assert_eq!(calculate_column("test\n"), 0);
        assert_eq!(calculate_column("test\ntest2"), 5);
        assert_eq!(calculate_column("test\ntest中2"), 6);
        assert_eq!(calculate_column("test\ntest中2\n中3"), 2);
    }

    #[test]
    fn test_tokens_from_source_basic() {
        let source = "/* test */\n// test";
        let mut tokens = dbg!(Vec::from_iter(source_to_iter(source))).into_iter();
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 1,
                column: 0,
                ..
            })
        ); // Block comment
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 2,
                column: 0,
                ..
            })
        ); // Line comment
    }

    #[test]
    fn test_tokens_with_line_column_values_set_correctly_more_unicode() {
        let source = "/* te中st */\n// test";
        let mut tokens = source_to_iter(source);
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 1,
                column: 0,
                ..
            })
        ); // Block comment
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 2,
                column: 0,
                ..
            })
        ); // Line comment
    }

    #[test]
    fn test_tokens_with_line_column_values_set_correctly_another() {
        let source = "/* te中st */\n// test\nfn 中(){\t}";
        let mut tokens = source_to_iter(source);
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 1,
                column: 0,
                ..
            })
        ); // Block comment
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 2,
                column: 0,
                ..
            })
        ); // Block comment
    }

    #[test]
    fn test_tokens_retain_empty_lines_for_clustering() {
        let source = r###"// ```c
// space:
//
// end
// ```
"###;
        let mut tokens = source_to_iter(source);
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 1,
                column: 0,
                ..
            })
        );
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 2,
                column: 0,
                ..
            })
        );
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 3,
                column: 0,
                ..
            })
        );
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 4,
                column: 0,
                ..
            })
        );
        assert_matches!(
            tokens.next(),
            Some(TokenWithType {
                line: 5,
                column: 0,
                ..
            })
        );
    }

    #[test]
    fn test_identify_token_type_assigns_block_comment_type_to_block_comments() {
        let block_comments = vec![
            TokenWithLineColumn {
                content: "/* Block Comment */".to_string(),
                line: 0,
                column: 0,
            },
            TokenWithLineColumn {
                content: "/* Multiple Line\nBlock Comment */".to_string(),
                line: 0,
                column: 0,
            },
        ];
        for token in block_comments {
            assert_eq!(TokenWithType::from(token).kind, TokenType::BlockComment);
        }
    }

    #[test]
    fn test_identify_token_type_assigns_line_comment_type_to_line_comments() {
        let line_comments = vec![TokenWithLineColumn {
            content: "// Line Comment ".to_string(),
            line: 0,
            column: 0,
        }];
        for token in line_comments {
            assert_eq!(TokenWithType::from(token).kind, TokenType::LineComment);
        }
    }

    /// Convenience function to create a single `TokenWithLineColumn` with given
    /// string content at line 0 and column 0
    fn token_with_line_column_at_start(content: &str) -> TokenWithLineColumn {
        TokenWithLineColumn {
            content: content.to_string(),
            line: 0,
            column: 0,
        }
    }

    #[test]
    fn test_identify_token_type_assigns_other_type_to_non_developer_comments() {
        let not_developer_comments = vec![
            token_with_line_column_at_start("/// Outer documentation comment"),
            token_with_line_column_at_start("//! Inner documentation comment"),
        ];
        for token in not_developer_comments {
            assert_eq!(TokenWithType::from(token).kind, TokenType::Other);
        }
    }

    fn concatenate_with_line_breaks(includes: &[&str], excludes: &[&str]) -> String {
        let mut building = String::new();
        for piece in includes {
            building = building + piece + "\n";
        }
        for piece in excludes {
            building = building + piece + "\n"
        }
        building
    }

    #[test]
    fn retain_only_developer_comments_removes_non_comment_tokens() {
        let includes = vec!["/* A block comment */", "// A line comment"];
        let excludes = vec![
            "fn", "func中", "(", ")", "{", "1", "+", "2", ";", "}", "\n", " ",
        ];
        let source = concatenate_with_line_breaks(&includes, &excludes);
        let tokens = source_to_iter(&source);
        for token in tokens {
            for content in &excludes {
                assert_ne!(&token.content, content);
            }
        }
    }

    #[test]
    fn retain_only_developer_comments_removes_documentation_comment_tokens() {
        let includes = vec!["/* A block comment */", "// A line comment"];
        let excludes = vec![
            "//! An inner documentation comment",
            "/// An outer documentation comment",
        ];
        let source = concatenate_with_line_breaks(&includes, &excludes);
        let tokens = source_to_iter(&source);
        for token in tokens {
            for content in &excludes {
                assert_ne!(&token.content, content);
            }
        }
    }

    #[test]
    fn retain_only_developer_comments_keeps_developer_comment_tokens() {
        let includes = vec!["/* A block comment */", "// A line comment"];
        let excludes = vec![
            "fn", "func中", "(", ")", "{", "1", "+", "2", ";", "}", "\n", " ",
        ];
        let source = concatenate_with_line_breaks(&includes, &excludes);
        let tokens = source_to_iter(&source).collect::<Vec<_>>();
        for content in includes {
            let tokens = tokens
                .iter()
                .filter(|t| t.content == content)
                .collect::<Vec<_>>();
            assert!(!tokens.is_empty());
        }
    }

    #[test]
    fn test_block_comments_to_literal_sets_converter_keeps_block_comment_tokens() {
        let source = "/* block comment */\n/*\n * multi line block comment\n */\n";
        let tokens = source_to_iter(source);
        let literal_sets = construct_literal_sets(tokens);
        assert_eq!(literal_sets.len(), 2);
    }

    #[test]
    fn test_block_comments_to_literal_sets_converter_ignores_other_token_types() {
        let source = "/// line comment\n/// outer documentation\npub fn test() -> i32 \
        {\n  //! inner documentation\n  1 + 2\n}";
        let tokens = source_to_iter(source);
        let literal_sets = construct_literal_sets(tokens);
        assert_eq!(literal_sets.len(), 0);
    }

    #[test]
    fn test_single_line_block_comment_literal_correctly_created() {
        let source = "/* block 种 comment */";
        let tokens = source_to_iter(source).collect::<Vec<_>>();
        assert_eq!(tokens.len(), 1);
        let token = tokens.last().unwrap();
        let literal_set = literal_set_from_block_comment(token);
        assert!(literal_set.is_ok());
        let literal_set = literal_set.unwrap();
        assert_eq!(literal_set.len(), 1);
        let literal = literal_set.literals().into_iter().last().unwrap();
        assert_eq!(literal.pre(), TokenType::BlockComment.pre_in_chars());
        assert_eq!(literal.post(), TokenType::BlockComment.post_in_chars());
        assert_eq!(literal.len_in_chars(), source.chars().count() - 4);
        assert_eq!(literal.len(), source.len() - 4);
        let span = &literal.span();
        assert_eq!(span.start.line, 1);
        assert_eq!(span.start.column, 2);
        assert_eq!(span.end.line, 1);
        assert_eq!(span.end.column, source.chars().count() - 2 - 1);
    }

    #[test]
    fn test_single_line_indented_block_comment_literal_correctly_created() {
        let source = "    /* block 种 comment */";
        let tokens = source_to_iter(source).collect::<Vec<_>>();
        assert!(tokens.len() > 0);
        let token = tokens.last().unwrap();
        let literal_set = literal_set_from_block_comment(&token);
        assert!(literal_set.is_ok());
        let literal_set = literal_set.unwrap();
        assert_eq!(literal_set.len(), 1);
        let literal = literal_set.literals().into_iter().last().unwrap();
        let indent_size = "    ".len(); // Also chars, because ASCII
        assert_eq!(literal.pre(), TokenType::BlockComment.pre_in_chars());
        assert_eq!(literal.post(), TokenType::BlockComment.post_in_chars());
        assert_eq!(
            literal.len_in_chars(),
            source.chars().count() - indent_size - 4
        );
        assert_eq!(literal.len(), source.len() - indent_size - 4);
        let span = &literal.span();
        assert_eq!(span.start.line, 1);
        assert_eq!(span.start.column, indent_size + 2);
        assert_eq!(span.end.line, 1);
        assert_eq!(span.end.column, source.chars().count() - 2 - 1);
    }

    #[test]
    fn test_multi_line_block_comment_literal_correctly_created() {
        let source = "/* block\n 种 \ncomment */";
        let tokens = source_to_iter(source).collect::<Vec<_>>();
        assert_eq!(tokens.len(), 1);
        let token = tokens.into_iter().last().unwrap();
        let literal_set = literal_set_from_block_comment(&token);
        assert!(literal_set.is_ok());
        let literal_set = literal_set.unwrap();
        assert_eq!(literal_set.len(), 3);
        let literals = literal_set.literals();
        {
            let literal = literals.get(0).unwrap();
            assert_eq!(literal.pre(), TokenType::BlockComment.pre_in_chars());
            assert_eq!(literal.post(), "".chars().count());
            assert_eq!(literal.len_in_chars(), " block".chars().count());
            assert_eq!(literal.len(), " block".len());
            let span = &literal.span();
            assert_eq!(span.start.line, 1);
            assert_eq!(span.start.column, 2);
            assert_eq!(span.end.line, 1);
            assert_eq!(span.end.column, "/* block".chars().count() - 1);
        }
        {
            let literal = literals.get(1).unwrap();
            assert_eq!(literal.pre(), "".chars().count());
            assert_eq!(literal.post(), "".chars().count());
            assert_eq!(literal.len_in_chars(), " 种 ".chars().count());
            assert_eq!(literal.len(), " 种 ".len());
            let span = &literal.span();
            assert_eq!(span.start.line, 2);
            assert_eq!(span.start.column, 0);
            assert_eq!(span.end.line, 2);
            assert_eq!(span.end.column, " 种 ".chars().count() - 1);
        }
        {
            let literal = literals.get(2).unwrap();
            assert_eq!(literal.pre(), "".chars().count());
            assert_eq!(literal.post(), TokenType::BlockComment.post_in_chars());
            assert_eq!(literal.len_in_chars(), "comment ".chars().count());
            assert_eq!(literal.len(), "comment ".len());
            let span = &literal.span();
            assert_eq!(span.start.line, 3);
            assert_eq!(span.start.column, 0);
            assert_eq!(span.end.line, 3);
            assert_eq!(span.end.column, "comment ".chars().count() - 1);
        }
    }

    #[test]
    fn outer_inner_mix() {
        let source = "// line comment\n/// Outer documentation\nfn test(){\n \
        //! Inner documentation\n\tlet i = 1 + 2;\n}";
        let tokens = source_to_iter(source);
        let sets = construct_literal_sets(tokens);
        // we only track dev comments
        assert_eq!(sets.len(), 1);
    }

    #[test]
    fn test_non_line_comment_tokens_line_comment_to_literal_does_not_create_literals() {
        let source = "/* Block comment */\nfn test(i: usize) {\n  let j = 1 + i;\n  j\n}";
        let tokens = source_to_iter(source);
        for token in tokens {
            assert!(literal_from_line_comment(&token).is_err());
        }
    }

    #[test]
    fn test_documentation_line_comment_tokens_line_comment_to_literal_does_not_create_literals() {
        let source = "/// Outer \nfn(){\n//! Inner \n}";
        let tokens = source_to_iter(source);
        for token in tokens {
            assert!(literal_from_line_comment(&token).is_err());
        }
    }

    #[test]
    fn test_developer_line_comment_tokens_line_comment_to_literal_create_literals_with_correct_data(
    ) {
        let source = "// First line comment\nconst ZERO: usize = 0; // A constant ";
        let filtered = source_to_iter(source).collect::<Vec<_>>();
        assert_eq!(filtered.len(), 2);
        let literals: Vec<std::result::Result<TrimmedLiteral, String>> = filtered
            .into_iter()
            .map(|t| literal_from_line_comment(&t))
            .collect();
        {
            let literal = literals.get(0).unwrap();
            assert!(literal.is_ok());
            let literal = literal.as_ref().unwrap();
            assert_eq!(literal.pre(), TokenType::LineComment.pre_in_chars());
            assert_eq!(literal.post(), TokenType::LineComment.post_in_chars());
            assert_eq!(
                literal.len_in_chars(),
                " First line comment".chars().count()
            );
            assert_eq!(literal.len(), " First line comment".len());
            let span = &literal.span();
            assert_eq!(span.start.line, 1);
            assert_eq!(span.start.column, 2);
            assert_eq!(span.end.line, 1);
            assert_eq!(
                span.end.column,
                2 + " First line comment".chars().count() - 1
            );
        }
        {
            let literal = literals.get(1).unwrap();
            assert!(literal.is_ok());
            let literal = literal.as_ref().unwrap();
            assert_eq!(literal.pre(), TokenType::LineComment.pre_in_chars());
            assert_eq!(literal.post(), TokenType::LineComment.post_in_chars());
            assert_eq!(literal.len_in_chars(), " A constant ".chars().count());
            assert_eq!(literal.len(), " A constant ".len());
            let span = &literal.span();
            assert_eq!(span.start.line, 2);
            assert_eq!(span.start.column, 25);
            assert_eq!(span.end.line, 2);
            assert_eq!(span.end.column, 25 + " A constant ".chars().count() - 1);
        }
    }

    #[test]
    fn test_single_line_comment_put_in_one_literal_set() {
        let content = " line comment";
        let source = format!("//{content}");
        let tokens = source_to_iter(&source);
        let literal_sets = construct_literal_sets(tokens);
        assert_eq!(literal_sets.len(), 1);
        let literal_set = literal_sets.get(0).unwrap();
        let all_literals = literal_set.literals();
        let literal = all_literals.get(0);
        assert!(literal.is_some());
        let literal = literal.unwrap();
        assert!(literal.as_str().contains(content));
    }

    #[test]
    fn test_adjacent_line_comments_put_in_same_literal_set() {
        let content_1 = " line comment 1 ";
        let content_2 = " line comment 2 ";
        let source = format!("//{content_1}\n//{content_2}");
        let tokens = source_to_iter(&source);
        let literal_sets = construct_literal_sets(tokens);
        assert_eq!(literal_sets.len(), 1);
        let literal_set = literal_sets.get(0).unwrap();
        let all_literals = literal_set.literals();
        assert_eq!(all_literals.len(), 2);
        {
            let literal = all_literals.get(0).unwrap();
            assert!(literal.as_str().contains(content_1));
        }
        {
            let literal = all_literals.get(1).unwrap();
            assert!(literal.as_str().contains(content_2));
        }
    }

    #[test]
    fn test_non_adjacent_line_comments_put_in_different_literal_sets() {
        let content_1 = " line comment 1 ";
        let content_2 = " line comment 2 ";
        let source = format!("//{content_1}\nfn(){{}}\n//{content_2}");
        let tokens = source_to_iter(&source);
        let literal_sets = construct_literal_sets(tokens);
        assert_eq!(literal_sets.len(), 2);
        {
            let literal_set = literal_sets.get(0).unwrap();
            let all_literals = literal_set.literals();
            assert_eq!(all_literals.len(), 1);
            let literal = all_literals.get(0).unwrap();
            assert!(literal.as_str().contains(content_1));
        }
        {
            let literal_set = literal_sets.get(1).unwrap();
            let all_literals = literal_set.literals();
            assert_eq!(all_literals.len(), 1);
            let literal = all_literals.get(0).unwrap();
            assert!(literal.as_str().contains(content_2));
        }
    }
}
