
use ra_ap_syntax::{SyntaxNode, SourceFile, tokenize, Token};
use regex::Regex;

use super::*;

#[derive(Debug)]
pub struct TokenWithLocation {
  content: String,
  location: usize
}

#[derive(Debug)]
struct TokenWithLineColumn {
  content: String,
  line: usize,
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

pub fn count_lines(fragment: &str) -> usize {
  fragment.chars().into_iter().filter(|c| *c == '\n').count() + 1
}

pub fn calculate_column(fragment: &str) -> usize {
  match fragment.rfind('\n') {
    Some(p) => fragment.chars().count() - fragment[..p].chars().count(),
    None => fragment.chars().count() + 1
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
    // Note: next column after last, in chars
    assert_eq!(calculate_column(""), 1);
    assert_eq!(calculate_column("test中"), 6);
    assert_eq!(calculate_column("test\n"), 1);
    assert_eq!(calculate_column("test\ntest2"), 6);
    assert_eq!(calculate_column("test\ntest中2"), 7);
    assert_eq!(calculate_column("test\ntest中2\n中3"), 3);
   }

  #[test]
  fn source_to_token_with_location_calculates_correct_locations() {
    // Note: next column after last, in chars
    {
      let tokens = source_to_tokens_with_location("/* test */\n// test");
      assert_eq!(tokens.get(0).unwrap().location, 0);
      assert_eq!(tokens.get(1).unwrap().location, 10);
    }
    {
      let tokens = source_to_tokens_with_location("/* te中st */\n\\ test");
      assert_eq!(tokens.get(0).unwrap().location, 0);
      assert_eq!(tokens.get(1).unwrap().location, 13);
    }
    {
      let tokens = source_to_tokens_with_location("/* te中st */\n// test\nfn 中(){\t}");
      println!("{:?}", tokens);
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
}