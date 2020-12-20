//! Check links of comments if they actually exist.

use anyhow::{anyhow, Result};

use crate::checker::Checker;
use crate::documentation::{CheckableChunk, Documentation};
use crate::util::{
    byte_range_to_char_range, byte_range_to_char_range_many, load_span_from, sub_char_range,
};
use crate::{CommentVariant, ContentOrigin, Detector, Range, Span, LineColumn, Suggestion, SuggestionSet};


mod config;
pub use config::LinkCheckConfig;

use tokio::prelude::*;
use tokio::runtime::Runtime;

use futures::stream::FuturesUnordered;
use futures::StreamExt;

#[derive(Debug)]
pub struct LinkCheck;

impl Checker for LinkCheck {
    type Config = LinkCheckConfig;
    fn check<'a, 's>(docu: &'a Documentation, config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        let client = lychee::ClientBuilder::default()
            .exclude_private_ips(config.exclude_private_ips)
            .build().unwrap();

        use std::pin::Pin;

        let mut unordered: FuturesUnordered<Pin<Box<dyn futures::Future<Output=Result<SuggestionSet<'s>>>>>> = FuturesUnordered::new();
        for (origin, chunks) in docu.iter() {
            for chunk in chunks {
                let client = client.clone();
                let fut = Box::pin(async move {
                    let ic = lychee::collector::InputContent::from_string(chunk.as_str(), lychee::extract::FileType::Markdown);

                    let links = lychee::collector::collect_links(
                        &[ic.input],
                        None,
                        true,
                        5_usize,
                    ).await.unwrap();

                    let mut suggestion_set = SuggestionSet::<'s>::new();
                    for link in links {
                        let response = client.check(link).await;
                        if !response.status.is_success() {
                            let suggestion = Suggestion::<'s> {
                                detector: Detector::Lychee,
                                chunk,
                                origin: origin.clone(),
                                span: Span {
                                    start: LineColumn {
                                        line: 1usize,
                                        column: 1usize,
                                    },
                                    end: LineColumn {
                                        line: 1usize,
                                        column: 1usize,
                                    }
                                }, // FIXME
                                range: 0..1,
                                replacements: vec![],
                                description: None,
                            };
                            suggestion_set.add(origin.clone(), suggestion);
                        }
                    }
                    Ok(suggestion_set)
                });
                unordered.push(fut);
            }
        }

        let rt  = Runtime::new()?;

        rt.block_on(async {
            unordered.into_future().await;
        });

        anyhow::bail!("join all future sets")
    }
}
