
use ra_ap_syntax::{SyntaxNode, SourceFile, tokenize, Token};
use regex::Regex;

use super::*;

#[derive(Debug)]
pub struct TokenWithLocation {
  content: String,
  // Bytes
  location: usize
}

#[derive(Debug)]
pub struct TokenWithLineColumn {
  content: String,
  line: usize,
  // Characters
  column: usize
}

pub fn source_to_tokens_with_location(source: &str) -> Vec<TokenWithLocation> {
  let ra_tokens = tokenize(source).0;
  let mut tokens = vec!();
  let mut location = 0;
  for token in ra_tokens {
    let length = usize::from(token.len);
    tokens.push(TokenWithLocation{
      content: source[location..location + length].to_string(),
      location
    });
    location += length;
  }
  tokens
}

pub fn tokens_with_location_to_tokens_with_line_and_column
(source: &str, tokens_in: Vec<TokenWithLocation>) -> Vec<TokenWithLineColumn> {
  let mut tokens_out = vec!();
  for token in tokens_in {
    tokens_out.push(TokenWithLineColumn{
      content: token.content,
      line: count_lines(&source[..token.location]),
      column: calculate_column(&source[..token.location])
    });
  }
  tokens_out
}

pub fn count_lines(fragment: &str) -> usize {
  fragment.chars().into_iter().filter(|c| *c == '\n').count() + 1
}

pub fn calculate_column(fragment: &str) -> usize {
  match fragment.rfind('\n') {
    Some(p) => fragment.chars().count() - fragment[..p].chars().count() - 1,
    None => fragment.chars().count()
  }
}

#[cfg(test)]
mod tests {
  use crate::documentation::developer::*;
  use ra_ap_syntax::tokenize;

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
  fn test_source_to_token_with_location_calculates_correct_locations() {
    {
      let tokens = source_to_tokens_with_location("/* test */\n// test");
      assert_eq!(tokens.get(0).unwrap().location, 0); // Block comment
      assert_eq!(tokens.get(1).unwrap().location, 10); // Whitespace
      assert_eq!(tokens.get(2).unwrap().location, 11); // Line comment
    }
    {
      let tokens = source_to_tokens_with_location("/* te中st */\n// test");
      assert_eq!(tokens.get(0).unwrap().location, 0); // Block comment
      assert_eq!(tokens.get(1).unwrap().location, 13); // Whitespace
      assert_eq!(tokens.get(2).unwrap().location, 14); // Line comment
    }
    {
      let tokens = source_to_tokens_with_location("/* te中st */\n// test\nfn 中(){\t}");
      assert_eq!(tokens.get(0).unwrap().location, 0); // Block comment
      assert_eq!(tokens.get(1).unwrap().location, 13); // Whitespace
      assert_eq!(tokens.get(2).unwrap().location, 14); // Line comment
      assert_eq!(tokens.get(3).unwrap().location, 21); // Whitespace
      assert_eq!(tokens.get(4).unwrap().location, 22); // Function keyword
      assert_eq!(tokens.get(5).unwrap().location, 24); // Whitespace
      assert_eq!(tokens.get(6).unwrap().location, 25); // Function name
      assert_eq!(tokens.get(7).unwrap().location, 28); // Open bracket
      assert_eq!(tokens.get(8).unwrap().location, 29); // Close bracket
      assert_eq!(tokens.get(9).unwrap().location, 30); // Open curly bracket
      assert_eq!(tokens.get(10).unwrap().location, 31); // Whitespace
      assert_eq!(tokens.get(11).unwrap().location, 32); // Close curly bracket
    }
  }

  #[test]
  fn test_tokens_with_line_column_values_set_correctly() {
    {
      let source = "/* test */\n// test";
      let tokens = source_to_tokens_with_location(source);
      let tokens = tokens_with_location_to_tokens_with_line_and_column(
        source, tokens);
      assert_eq!(tokens.get(0).unwrap().line, 1); // Block comment
      assert_eq!(tokens.get(0).unwrap().column, 0);
      assert_eq!(tokens.get(1).unwrap().line, 1); // Whitespace
      assert_eq!(tokens.get(1).unwrap().column, 10);
      assert_eq!(tokens.get(2).unwrap().line, 2); // Line comment
      assert_eq!(tokens.get(2).unwrap().column, 0);
    }
    {
      let source = "/* te中st */\n// test";
      let tokens = source_to_tokens_with_location(source);
      let tokens = tokens_with_location_to_tokens_with_line_and_column(
        source, tokens);
      assert_eq!(tokens.get(0).unwrap().line, 1); // Block comment
      assert_eq!(tokens.get(0).unwrap().column, 0);
      assert_eq!(tokens.get(1).unwrap().line, 1); // Whitespace
      assert_eq!(tokens.get(1).unwrap().column, 11);
      assert_eq!(tokens.get(2).unwrap().line, 2); // Line comment
      assert_eq!(tokens.get(2).unwrap().column, 0);
    }
    {
      let source = "/* te中st */\n// test\nfn 中(){\t}";
      let tokens = source_to_tokens_with_location(source);
      let tokens = tokens_with_location_to_tokens_with_line_and_column(
        source, tokens);
      assert_eq!(tokens.get(0).unwrap().line, 1); // Block comment
      assert_eq!(tokens.get(0).unwrap().column, 0);
      assert_eq!(tokens.get(1).unwrap().line, 1); // Whitespace
      assert_eq!(tokens.get(1).unwrap().column, 11);
      assert_eq!(tokens.get(2).unwrap().line, 2); // Line comment
      assert_eq!(tokens.get(2).unwrap().column, 0);
      assert_eq!(tokens.get(3).unwrap().line, 2); // Whitespace
      assert_eq!(tokens.get(3).unwrap().column, 7);
      assert_eq!(tokens.get(4).unwrap().line, 3); // Function keyword
      assert_eq!(tokens.get(4).unwrap().column, 0);
      assert_eq!(tokens.get(5).unwrap().line, 3); // Whitespace
      assert_eq!(tokens.get(5).unwrap().column, 2);
      assert_eq!(tokens.get(6).unwrap().line, 3); // Function name
      assert_eq!(tokens.get(6).unwrap().column, 3);
      assert_eq!(tokens.get(7).unwrap().line, 3); // Open bracket
      assert_eq!(tokens.get(7).unwrap().column, 4);
      assert_eq!(tokens.get(8).unwrap().line, 3); // Close bracket
      assert_eq!(tokens.get(8).unwrap().column, 5);
      assert_eq!(tokens.get(9).unwrap().line, 3); // Open curly bracket
      assert_eq!(tokens.get(9).unwrap().column, 6);
      assert_eq!(tokens.get(10).unwrap().line, 3); // Whitespace
      assert_eq!(tokens.get(10).unwrap().column, 7);
      assert_eq!(tokens.get(11).unwrap().line, 3); // Close curly bracket
      assert_eq!(tokens.get(11).unwrap().column, 8);
    }
  }
}