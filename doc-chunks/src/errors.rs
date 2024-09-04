use crate::{Range, Span};
use indexmap::IndexMap;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Really pretty much anything")]
    Any,

    #[error("Failed to parse rust content: {0:?}")]
    ParserFailure(#[source] syn::Error),

    #[error("Failed to parse toml file")]
    Toml(#[from] toml::de::Error),

    #[error("{0}")]
    Span(String),

    #[error("BUG: Found a range {}..{} which that does not exist in its own source mapping: {:?}", .line_range.start, .line_range.end, .source_mapping)]
    InvalidLineRange {
        line_range: Range,
        source_mapping: IndexMap<Range, Span>,
    },
}
