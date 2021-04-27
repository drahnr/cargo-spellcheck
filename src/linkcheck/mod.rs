//! Check links of comments if they lead anywhere.

use crate::errors::*;

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};
use crate::util::{
    byte_range_to_char_range, byte_range_to_char_range_many, load_span_from, sub_char_range,
};
use crate::{CommentVariant, ContentOrigin, Detector, Range, Span, LineColumn, Suggestion, SuggestionSet};


mod config;
pub use config::LinkCheckConfig;

#[derive(Debug)]
pub struct LinkCheck;

impl Checker for LinkCheck {
    type Config = crate::config::LinkCheckConfig;

    fn detector() -> Detector {
        Detector::LinkCheck
    }

    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {

        // for (origin, chunks) in docu.iter() {
        //     for chunk in chunks {
        //         {
        //             let mut suggestion_set = SuggestionSet::<'s>::new();
        //             for link in links {
        //                 let response = client.check(link).await;
        //                 if !response.status.is_success() {
        //                     let suggestion = Suggestion::<'s> {
        //                         detector: Detector::LinkCheck,
        //                         chunk,
        //                         origin: origin.clone(),
        //                         span: Span {
        //                             start: LineColumn {
        //                                 line: 1usize,
        //                                 column: 1usize,
        //                             },
        //                             end: LineColumn {
        //                                 line: 1usize,
        //                                 column: 1usize,
        //                             }
        //                         }, // FIXME
        //                         range: 0..1,
        //                         replacements: vec![],
        //                         description: None,
        //                     };
        //                     suggestion_set.add(origin.clone(), suggestion);
        //                 }
        //             }
        //             Ok(suggestion_set)
        //         }
        //     }
        // }
        Ok(SuggestionSet::new())

    }
}
