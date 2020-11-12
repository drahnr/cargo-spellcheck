
use ra_ap_syntax::{SyntaxNode, SourceFile, tokenize, Token};
use regex::Regex;

use super::*;

struct TokenWithLocation {
  content: String,
  location: usize
}

#[derive(Debug)]
struct TokenWithLineColumn {
  content: String,
  line: usize,
  column: usize
}

pub fn count_lines(fragment: &str) -> usize {
  fragment.chars().into_iter().filter(|c| *c == '\n').count() + 1
}

pub fn calculate_column(fragment: &str) -> usize {
  println!("{:?} {:?}", fragment, fragment.rfind('\n'));
  match fragment.rfind('\n') {
    Some(p) => fragment.chars().count() - p,
    None => fragment.len() + 1
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
    // Note: next column after last
    assert_eq!(calculate_column(""), 1);
    assert_eq!(calculate_column("test"), 5);
    assert_eq!(calculate_column("test\n"), 1);
    assert_eq!(calculate_column("test\ntest2"), 6);
   }
}