//! Covers all user triggered actions (except for signals).

use super::*;
use anyhow::{anyhow, Result};
use log::{debug, trace};
use std::convert::TryInto;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Read, Write};

use std::path::PathBuf;

pub mod bandaid;
pub mod bandaidset;
pub mod interactive;

pub(crate) use bandaid::*;
pub(crate) use bandaidset::*;

use interactive::{UserPicked, UserSelection};

/// State of conclusion.
#[derive(Debug, Clone, Copy)]
pub enum Finish {
    /// Abort is user requested, either by signal or key stroke.
    Abort,
    /// Completion of the check run, with the resulting number of
    /// mistakes accumulated.
    MistakeCount(usize),
}

impl Finish {
    /// A helper to determine if any mistakes were found.
    pub fn found_any(&self) -> bool {
        match *self {
            Self::MistakeCount(n) if n > 0 => true,
            _ => false,
        }
    }
}

/// A patch to be stitched ontop of another string.
///
/// Has intentionally no awareness of any rust or cmark/markdown semantics.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum Patch {
    /// Replace the area spanned by `replace` with `replacement`.
    /// Since `Span` is inclusive, `Replace` always will replace a character in the original sources.
    Replace {
        replace_span: Span,
        replacement: String,
    },
    /// Location where to insert.
    Insert {
        insert_at: LineColumn,
        content: String,
    },
}

impl<'a> From<&'a BandAid> for Patch {
    fn from(bandaid: &'a BandAid) -> Self {
        // TODO XXX
        Self::from(bandaid.clone())
    }
}

impl From<BandAid> for Patch {
    fn from(bandaid: BandAid) -> Self {
        // TODO XXX this conversion is probably too simplistic
        match bandaid {
            BandAid::Deletion(span) => Self::Replace {
                replace_span: span.clone(),
                replacement: String::new(),
            },
            BandAid::Injection(linecol, content) => Self::Insert {
                insert_at: linecol,
                content: content.to_owned(),
            },
            BandAid::Replacement(span, content) => Self::Replace {
                replace_span: span.clone(),
                replacement: content.to_owned(),
            },
        }
    }
}

/// Correct all lines by applying bandaids.
///
/// Assumes all `BandAids` do not overlap when replacing.
/// Inserting multiple times at a particular `LineColumn` is ok,
/// but replacing overlapping `Span`s of the original source is not.
///
/// This function is not concerend with _any_ semantics or comments or
/// whatsoever at all.
///
/// Note that with the current implementation trailing newlines are NOT
/// preserved.
///
/// [https://github.com/drahnr/cargo-spellcheck/issues/116](Tracking issue).
fn correct_lines<'s, II, I>(patches: II, source_buffer: String, mut sink: impl Write) -> Result<()>
where
    II: IntoIterator<IntoIter = I, Item = Patch>,
    I: Iterator<Item = Patch>,
{
    let patches = patches.into_iter();
    let mut patches = patches.peekable();

    let mut source_iter =
        iter_with_line_column_from(source_buffer.as_str(), LineColumn { line: 1, column: 0 })
            .peekable();

    let mut current = None;
    let mut byte_cursor = 0usize;
    loop {
        let cc_from_byteoffset = if let Some(ref current) = current {
            let (cc_start, data) = match current {
                Patch::Replace {
                    replace_span,
                    replacement,
                } => (replace_span.end, replacement.as_str()),
                Patch::Insert { insert_at, content } => (insert_at.clone(), content.as_str()),
            };

            sink.write(data.as_bytes())?;

            // skip
            let mut cc_start_byte_offset = byte_cursor;
            'skip: while let Some((_c, byte_offset, _idx, linecol)) = source_iter.next() {
                cc_start_byte_offset = byte_offset;
                if linecol >= cc_start {
                    break 'skip;
                }
            }
            cc_start_byte_offset
        } else {
            byte_cursor
        };
        debug_assert!(byte_cursor <= cc_from_byteoffset);
        byte_cursor = cc_from_byteoffset;

        let cc_to_byteoffset = if let Some(upcoming) = patches.peek() {
            let cc_end = match upcoming {
                Patch::Replace { replace_span, .. } => replace_span.start,
                Patch::Insert { insert_at, .. } => insert_at.clone(),
            };

            // do not write anythin

            // carbon copy until this byte offset
            let mut cc_end_byte_offset = byte_cursor;
            'cc: while let Some((_c, byte_offset, _idx, linecol)) = source_iter.next() {
                if linecol >= cc_end {
                    break 'cc;
                }
                // we need to drag this one behind, since...
                cc_end_byte_offset = byte_offset;
            }
            // in the case we reach EOF here the `cc_end_byte_offset` could never be updated correctly
            std::cmp::min(cc_end_byte_offset + 1, source_buffer.len())
        } else {
            source_buffer.len()
        };
        debug_assert!(byte_cursor <= cc_to_byteoffset);

        byte_cursor = cc_to_byteoffset;

        let cc_range = cc_from_byteoffset..cc_to_byteoffset;
        sink.write(&source_buffer[cc_range].as_bytes())?;

        // move on to the next
        current = patches.next();

        if current.is_none() {
            // we already made sure earlier to write out everything
            break;
        }
    }

    Ok(())
}

/// Mode in which `cargo-spellcheck` operates
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Action {
    /// Only show errors
    Check,
    /// Interactively choose from checker provided suggestions.
    Fix,
    /// Reflow all commants to a given maximum column width.
    Reflow,
}

impl Action {
    /// Apply bandaids to the file represented by content origin.
    fn correction<'s>(
        self,
        origin: ContentOrigin,
        bandaids: impl IntoIterator<Item = BandAid>,
    ) -> Result<()> {
        match origin {
            ContentOrigin::CommonMarkFile(path) => self.correct_file(path, bandaids),
            ContentOrigin::RustSourceFile(path) => self.correct_file(path, bandaids),
            //TODO bandaids are relative to the doc-test, so fix the span with the one provided
            ContentOrigin::RustDocTest(path, _span) => self.correct_file(path, bandaids),
            #[cfg(test)]
            ContentOrigin::TestEntityRust => unreachable!("Use a proper file"),
            #[cfg(test)]
            ContentOrigin::TestEntityCommonMark => unreachable!("Use a proper file"),
        }
    }

    /// assumes suggestions are sorted by line number and column number and must be non overlapping
    fn correct_file<'s>(
        &self,
        path: PathBuf,
        bandaids: impl IntoIterator<Item = BandAid>,
    ) -> Result<()> {
        let path = path
            .as_path()
            .canonicalize()
            .map_err(|e| anyhow!("Failed to canonicalize {}", path.display()).context(e))?;
        let path = path.as_path();
        trace!("Attempting to open {} as read", path.display());
        let ro = std::fs::OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

        let mut reader = std::io::BufReader::new(ro);

        const TEMPORARY: &'static str = ".spellcheck.tmp";

        let tmp = std::env::current_dir()
            .expect("Must have cwd")
            .join(TEMPORARY);
        // let tmp = tmp.canonicalize().map_err(|e| { anyhow!("Failed to canonicalize {}", tmp.display() ).context(e) })?;
        //trace!("Attempting to open {} as read", tmp.display());
        let wr = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&tmp)
            .map_err(|e| anyhow!("Failed to open {}", path.display()).context(e))?;

        let mut writer = std::io::BufWriter::with_capacity(1024, wr);

        let mut content = String::with_capacity(2e6 as usize);
        reader.get_mut().read_to_string(&mut content)?;

        correct_lines(
            bandaids.into_iter().map(|x| Patch::from(x)),
            content, // FIXME for efficiency, correct_lines should integrate with `BufRead` instead of a `String` buffer
            &mut writer,
        )?;

        writer.flush()?;

        fs::rename(tmp, path)?;

        Ok(())
    }

    /// Consumingly apply the user picked changes to a file.
    ///
    /// **Attention**: Must be consuming, repeated usage causes shifts in spans and
    /// would destroy the file structure!
    pub fn write_changes_to_disk(&self, userpicked: UserPicked, _config: &Config) -> Result<()> {
        if userpicked.total_count() > 0 {
            debug!("Writing changes back to disk");
            for (path, bandaids) in userpicked.bandaids.into_iter() {
                self.correction(path, bandaids.into_iter())?;
            }
        } else {
            debug!("No band aids to apply");
        }
        Ok(())
    }

    /// Purpose was to check, checking complete, so print the results.
    fn check(&self, suggestions_per_path: SuggestionSet, _config: &Config) -> Result<Finish> {
        let mut count = 0_usize;
        for (_path, suggestions) in suggestions_per_path {
            count += suggestions.len();
            for suggestion in suggestions {
                println!("{}", suggestion);
            }
        }
        Ok(Finish::MistakeCount(count))
    }

    /// Run the requested action.
    pub fn run(self, suggestions: SuggestionSet, config: &Config) -> Result<Finish> {
        match self {
            Self::Check => self.check(suggestions, config),
            Self::Fix | Self::Reflow => {
                let (picked, user_sel) =
                    interactive::UserPicked::select_interactive(suggestions, config)?;
                if user_sel == UserSelection::Abort {
                    Ok(Finish::Abort)
                } else {
                    let n = picked.total_count();
                    self.write_changes_to_disk(picked, config)?;
                    Ok(Finish::MistakeCount(n))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! verify_correction {
        ($text:literal, $bandaids:expr, $expected:literal) => {
            let mut sink: Vec<u8> = Vec::with_capacity(1024);

            correct_lines(
                $bandaids.into_iter().map(|bandaid| Patch::from(bandaid)),
                $text.to_owned(),
                &mut sink,
            )
            .expect("Line correction must work in unit test!");

            assert_eq!(String::from_utf8_lossy(sink.as_slice()), $expected);
        };
    }

    #[test]
    fn plain() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let patches = vec![
            Patch::Replace {
                replace_span: Span {
                    start: LineColumn { line: 1, column: 6 },
                    end: LineColumn {
                        line: 2,
                        column: 13,
                    },
                },
                replacement: "& Omega".to_owned(),
            },
            Patch::Insert {
                insert_at: LineColumn { line: 3, column: 0 },
                content: "\nIcecream truck".to_owned(),
            },
        ];
        verify_correction!(
            r#"Alpha beta gamma
zeta eta beta.
"#,
            patches,
            r#"Alpha & Omega.

Icecream truck"#
        );
    }
    #[test]
    fn replace_unicorns() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let bandaids = vec![
            BandAid::Replacement(
                (2_usize, 7..15).try_into().unwrap(),
                "banana icecream".to_owned(),
            ),
            BandAid::Replacement((2_usize, 22..28).try_into().unwrap(), "third".to_owned()),
            BandAid::Replacement((2_usize, 29..36).try_into().unwrap(), "day".to_owned()),
        ];
        verify_correction!(
            r#"
I like unicorns every second Mondays.

"#,
            bandaids,
            r#"
I like banana icecream every third day.

"#
        );
    }

    #[test]
    fn bandaid_multiline() {
        let bandaids = vec![
            BandAid::Replacement(
                (2_usize, 27..36).try_into().unwrap(),
                "comments with".to_owned(),
            ),
            BandAid::Replacement(
                (3_usize, 0..17).try_into().unwrap(),
                " different multiple".to_owned(),
            ),
            BandAid::Replacement((3_usize, 18..23).try_into().unwrap(), "words".to_owned()),
        ];
        verify_correction!(
            "
/// Let's test bandaids on comments
/// with multiple lines afterwards
",
            bandaids,
            "
/// Let's test bandaids on comments with
/// different multiple words afterwards
"
        );
    }

    #[test]
    fn bandaid_deletion() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();
        let bandaids = vec![
            BandAid::Replacement(
                (2_usize, 27..36).try_into().unwrap(),
                "comments with multiple words".to_owned(),
            ),
            BandAid::Deletion((3_usize, 0..17).try_into().unwrap()),
        ];
        verify_correction!(
            "
/// Let's test bandaids on comments
/// with multiple lines afterwards
",
            bandaids,
            "
/// Let's test bandaids on comments with multiple words
"
        );
    }

    #[test]
    fn bandaid_injection() {
        let bandaids = vec![
            BandAid::Replacement(
                (2_usize, 27..36).try_into().unwrap(),
                "comments with multiple words".to_owned(),
            ),
            BandAid::Injection(
                LineColumn {
                    line: 3_usize,
                    column: 0,
                },
                " but still more content".to_owned(),
            ),
        ];
        verify_correction!(
            "
/// Let's test bandaids on comments
/// with multiple lines afterwards
",
            bandaids,
            "
/// Let's test bandaids on comments with multiple words
/// but still more content
/// with multiple lines afterwards
"
        );
    }

    #[test]
    fn bandaid_macrodoceq_injection() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let bandaids = vec![
            BandAid::Replacement((2_usize, 18..24).try_into().unwrap(), "uchen".to_owned()),
            BandAid::Injection(
                LineColumn {
                    line: 2_usize,
                    column: 24,
                },
                "für".to_owned(),
            ),
            BandAid::Injection(
                LineColumn {
                    line: 2_usize,
                    column: 24,
                },
                "den".to_owned(),
            ),
            BandAid::Deletion((2_usize, 24..25).try_into().unwrap()),
            BandAid::Deletion((3_usize, 0..10).try_into().unwrap()),
        ];
        verify_correction!(
            r#"
#[ doc = "Erdbeerkompott
          Eisbär"]
"#,
            bandaids,
            r#"
#[ doc = "Erdbeerkuchen"]
#[ doc = "für"]
#[ doc = "den"]
#[ doc = "Eisbär"]
"#
        );
    }
}
