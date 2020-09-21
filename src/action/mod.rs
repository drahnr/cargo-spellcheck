//! Covers all user triggered actions (except for signals).

use super::*;
use anyhow::{anyhow, Result};
use log::{debug, trace};
use std::convert::TryInto;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Write};

use std::path::PathBuf;

pub mod bandaid;
pub mod interactive;

pub(crate) use bandaid::*;
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

/// correct all lines
/// `bandaids` are the fixes to be applied to the lines
///
/// Note that `Lines` as created by `(x as BufLines).lines()` does
/// not preserve trailing newlines, so either the iterator
/// needs to be modified to yield an extra (i.e. with `.chain("".to_owned())`)
/// or a manual newlines has to be written to the `sink`.
fn correct_lines<'s>(
    mut bandaids: impl Iterator<Item = BandAid>,
    source: impl Iterator<Item = (usize, String)>,
    mut sink: impl Write,
) -> Result<()> {
    let mut nxt: Option<BandAid> = bandaids.next();
    for (line_number, content) in source {
        trace!("Processing line {}", line_number);
        let mut remainder_column = 0_usize;
        // let content: String = content.map_err(|e| {
        //     anyhow!("Line {} contains invalid utf8 characters", line_number).context(e)
        // })?;

        if nxt.is_none() {
            // no candidates remaining, just keep going
            sink.write(content.as_bytes())?;
            sink.write("\n".as_bytes())?;
            continue;
        }

        if let Some(ref bandaid) = nxt {
            if !bandaid.span.covers_line(line_number) {
                sink.write(content.as_bytes())?;
                sink.write("\n".as_bytes())?;
                continue;
            }
        }

        let content_len = content.chars().count();
        while let Some(bandaid) = nxt.take() {
            trace!("Applying next bandaid {:?}", bandaid);
            trace!("where line {} is: >{}<", line_number, content);
            let range: Range = bandaid
                .span
                .try_into()
                .expect("Bandaids must be single-line");

            // write prelude for this line between start or previous replacement
            if range.start > remainder_column {
                sink.write(util::sub_chars(&content, remainder_column..range.start).as_bytes())?;
            }
            // write the replacement chunk
            sink.write(bandaid.replacement.as_bytes())?;

            remainder_column = range.end;
            nxt = bandaids.next();
            let complete_current_line = if let Some(ref bandaid) = nxt {
                // if `nxt` is also targeting the current line, don't complete the line
                !bandaid.span.covers_line(line_number)
            } else {
                true
            };
            if complete_current_line {
                // the last replacement may be the end of content
                if remainder_column < content_len {
                    debug!(
                        "line {} len is {}, and remainder column is {}",
                        line_number, content_len, remainder_column
                    );
                    // otherwise write all
                    // not that this also covers writing a line without any suggestions
                    sink.write(
                        util::sub_chars(&content, remainder_column..content_len).as_bytes(),
                    )?;
                } else {
                    debug!(
                        "line {} len is {}, and remainder column is {}",
                        line_number, content_len, remainder_column
                    );
                }
                sink.write("\n".as_bytes())?;
                // break the inner loop
                break;
                // } else {
                // next suggestion covers same line
            }
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

        correct_lines(
            bandaids.into_iter(),
            (&mut reader)
                .lines()
                .filter_map(Result::ok)
                .enumerate()
                .map(|(lineno, content)| (lineno + 1, content)),
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

    const TEXT: &'static str = r#"
I like unicorns every second Mondays.

"#;

    const CORRECTED: &'static str = r#"
I like banana icecream every third day.

"#;

    #[test]
    fn replace_unicorns() {
        let _ = env_logger::Builder::new()
            .filter(None, log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let mut sink: Vec<u8> = Vec::with_capacity(1024);
        let bandaids = vec![
            BandAid {
                span: (2_usize, 7..15).try_into().unwrap(),
                replacement: "banana icecream".to_owned(),
            },
            BandAid {
                span: (2_usize, 22..28).try_into().unwrap(),
                replacement: "third".to_owned(),
            },
            BandAid {
                span: (2_usize, 29..36).try_into().unwrap(),
                replacement: "day".to_owned(),
            },
        ];

        let lines = TEXT
            .lines()
            .map(std::borrow::ToOwned::to_owned)
            .enumerate()
            .map(|(lineno, content)| (lineno + 1, content));

        correct_lines(bandaids.into_iter(), lines, &mut sink).expect("should be able to");

        assert_eq!(String::from_utf8_lossy(sink.as_slice()), CORRECTED);
    }

    #[test]
    fn bandaid_multiline() {
        const TEST: &'static str = "
/// Let's test bandaids on comments
/// with multiple lines

";

        const RESULT: &'static str = "
/// Let's test bandaids on comments with
/// different multiple lines
";

        let mut sink: Vec<u8> = Vec::with_capacity(1024);
        let bandaids = vec![
            BandAid {
                span: Span {
                    start: LineColumn {
                        line: 1,
                        column: 27,
                    },
                    end: LineColumn {
                        line: 2,
                        column: 17,
                    },
                },
                replacement: "comments with\n/// different multiple".to_owned(),
            },
            // BandAid {
            //     span: (2_usize, 22..28).try_into().unwrap(),
            //     replacement: "third".to_owned(),
            // },
            // BandAid {
            //     span: (2_usize, 29..36).try_into().unwrap(),
            //     replacement: "day".to_owned(),
            // },
        ];
        let lines = TEST
            .lines()
            .map(std::borrow::ToOwned::to_owned)
            .enumerate()
            .map(|(lineno, content)| (lineno + 1, content));

        correct_lines(bandaids.into_iter(), lines, &mut sink).expect("correction works. qed");
        assert_eq!(String::from_utf8_lossy(sink.as_slice()), RESULT);
    }
}
