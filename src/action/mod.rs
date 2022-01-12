//! Covers all user triggered actions (except for signals).

use super::*;
use crate::checker::Checkers;
use crate::errors::*;
use crate::reflow::Reflow;
use log::{debug, trace};

use fs_err as fs;
use futures::stream::{self, StreamExt, TryStreamExt};
use rayon::iter::ParallelIterator;

use std::io::{Read, Write};
use std::path::PathBuf;

pub mod bandaid;
pub mod interactive;

pub(crate) use bandaid::*;

use interactive::{UserPicked, UserSelection};

/// State of conclusion.
#[derive(Debug, Clone, Copy)]
pub enum Finish {
    /// Operation ran to the end, successfully.
    Success,
    /// Abort is user requested, either by signal or key stroke.
    Abort,
    /// Completion of the check run, with the resulting number of mistakes
    /// accumulated.
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

/// A patch to be stitched on-top of another string.
///
/// Has intentionally no awareness of any rust or cmark/markdown semantics.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum Patch {
    /// Replace the area spanned by `replace` with `replacement`. Since `Span`
    /// is inclusive, `Replace` will always replace a character in the original
    /// sources.
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
        match bandaid {
            bandaid if bandaid.span.start == bandaid.span.end => Self::Insert {
                insert_at: bandaid.span.start,
                content: bandaid.content,
            },
            _ => Self::Replace {
                replace_span: bandaid.span,
                replacement: bandaid.content,
            },
        }
    }
}

/// Correct lines by applying patches.
///
/// Assumes all `BandAids` do not overlap when replacing. Inserting multiple
/// times at a particular `LineColumn` is OK, but replacing overlapping `Span`s
/// of the original source is not.
///
/// This function is not concerned with _any_ semantics or comments or
/// whatsoever at all, it blindly replaces what is given to it.
pub(crate) fn apply_patches<'s, II, I>(
    patches: II,
    source_buffer: &str,
    mut sink: impl Write,
) -> Result<()>
where
    II: IntoIterator<IntoIter = I, Item = Patch>,
    I: Iterator<Item = Patch>,
{
    let mut patches = patches.into_iter().peekable();

    let mut source_iter =
        iter_with_line_column_from(source_buffer, LineColumn { line: 1, column: 0 }).peekable();

    const TARGET: &str = "patch";
    let mut write_to_sink = |topic: &str, data: &str| -> Result<()> {
        log::trace!(target: TARGET, "w<{}>: {}", topic, data.escape_debug());
        sink.write(data.as_bytes())?;
        Ok(())
    };

    let mut cc_end_byte_offset = 0;

    let mut current = None;
    let mut byte_cursor = 0usize;
    loop {
        let cc_start_byte_offset = if let Some(ref current) = current {
            let (cc_start, data, insertion) = match current {
                Patch::Replace {
                    replace_span,
                    replacement,
                } => (replace_span.end, replacement.as_str(), false),
                Patch::Insert { insert_at, content } => (insert_at.clone(), content.as_str(), true),
            };

            write_to_sink("new", data)?;

            if insertion {
                // do not advance anythin on insertion
                byte_cursor
            } else {
                // skip the range of chars based on the line column
                // so the cursor continues after the "replaced" characters
                let mut cc_start_byte_offset = byte_cursor;
                'skip: while let Some((c, byte_offset, _idx, linecol)) = source_iter.peek() {
                    let byte_offset = *byte_offset;
                    let linecol = *linecol;

                    cc_start_byte_offset = byte_offset + c.len_utf8();

                    if linecol >= cc_start {
                        log::trace!(
                            target: TARGET,
                            "skip buffer: >{}<",
                            &source_buffer[cc_end_byte_offset..cc_start_byte_offset].escape_debug()
                        );

                        break 'skip;
                    }

                    log::trace!(target: TARGET, "skip[{}]: >{}<", _idx, c.escape_debug());

                    let _ = source_iter.next();
                }
                cc_start_byte_offset
            }
        } else {
            byte_cursor
        };
        debug_assert!(byte_cursor <= cc_start_byte_offset);
        byte_cursor = cc_start_byte_offset;

        cc_end_byte_offset = if let Some(upcoming) = patches.peek() {
            let cc_end = match upcoming {
                Patch::Replace { replace_span, .. } => replace_span.start,
                Patch::Insert { insert_at, .. } => insert_at.clone(),
            };

            // do not write anything

            // carbon copy until this byte offset
            let mut cc_end_byte_offset = byte_cursor;
            'cc: while let Some((c, byte_offset, _idx, linecol)) = source_iter.peek() {
                let byte_offset = *byte_offset;
                let linecol = *linecol;

                if linecol >= cc_end {
                    log::trace!(
                        target: TARGET,
                        "copy buffer: >{}<",
                        &source_buffer[cc_start_byte_offset..cc_end_byte_offset].escape_debug()
                    );
                    break 'cc;
                }

                cc_end_byte_offset = byte_offset + c.len_utf8();

                log::trace!(target: TARGET, "copy[{}]: >{}<", _idx, c.escape_debug());

                let _ = source_iter.next();
                // we need to drag this one behind, since...
            }
            // in the case we reach EOF here the `cc_end_byte_offset` could never be updated correctly
            std::cmp::min(cc_end_byte_offset, source_buffer.len())
        } else {
            source_buffer.len()
        };
        debug_assert!(byte_cursor <= cc_end_byte_offset);

        byte_cursor = cc_end_byte_offset;

        let cc_range = cc_start_byte_offset..cc_end_byte_offset;

        write_to_sink("cc", &source_buffer[cc_range])?;

        // move on to the next
        current = patches.next();

        if current.is_none() {
            // we already made sure earlier to write out everything
            break;
        }
    }

    Ok(())
}

/// Mode in which `cargo-spellcheck` operates.
///
/// Eventually to be used directly in parsing arguments.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize)]
pub enum Action {
    /// Only show errors
    #[serde(alias = "check")]
    Check,

    /// Interactively choose from checker provided suggestions.
    #[serde(alias = "fix")]
    Fix,
    /// Reflow doc comments, so they adhere to a given maximum column width.
    #[serde(alias = "reflow")]
    Reflow,

    /// Print help and exit.
    #[serde(alias = "help")]
    Help,

    /// Print the version info and exit.
    #[serde(alias = "version")]
    Version,

    /// Print the config being in use, default config if none.
    #[serde(alias = "config")]
    Config,

    /// List all files in depth first sorted order in which they would be
    /// checked.
    #[serde(alias = "files")]
    ListFiles,
}

impl Action {
    /// Apply bandaids to the file represented by content origin.
    pub fn write_changes_to_disk(
        &self,
        origin: ContentOrigin,
        bandaids: impl IntoIterator<Item = BandAid>,
    ) -> Result<()> {
        match origin {
            ContentOrigin::CargoManifestDescription(path) => self.correct_file(path, bandaids),
            ContentOrigin::CommonMarkFile(path) => self.correct_file(path, bandaids),
            ContentOrigin::RustSourceFile(path) => self.correct_file(path, bandaids),
            ContentOrigin::RustDocTest(path, _span) => self.correct_file(path, bandaids),
            #[cfg(test)]
            ContentOrigin::TestEntityRust => unreachable!("Use a proper file"),
            #[cfg(test)]
            ContentOrigin::TestEntityCommonMark => unreachable!("Use a proper file"),
        }
    }

    /// assumes suggestions are sorted by line number and column number and must
    /// be non overlapping
    fn correct_file(
        &self,
        path: PathBuf,
        bandaids: impl IntoIterator<Item = BandAid>,
    ) -> Result<()> {
        let path = fs::canonicalize(path.as_path())?;
        let path = path.as_path();
        trace!("Attempting to open {} as read", path.display());
        let ro = fs::OpenOptions::new().read(true).open(path)?;

        let mut reader = std::io::BufReader::new(ro);

        const TEMPORARY: &str = ".spellcheck.tmp";

        // Avoid issues when processing multiple files in parallel
        let tmp_name = TEMPORARY.to_owned() + uuid::Uuid::new_v4().to_string().as_str();

        let tmp = std::env::current_dir()
            .expect("Must have cwd")
            .join(tmp_name);
        let wr = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&tmp)?;

        let mut writer = std::io::BufWriter::with_capacity(1024, wr);

        let mut content = String::with_capacity(2e6 as usize);
        reader.get_mut().read_to_string(&mut content)?;

        {
            let th = crate::TinHat::on();

            apply_patches(
                bandaids.into_iter().map(|x| Patch::from(x)),
                content.as_str(), // FIXME for efficiency, correct_lines should integrate with `BufRead` instead of a `String` buffer
                &mut writer,
            )?;

            writer.flush()?;

            fs::rename(tmp, path)?;

            // Writing for this file is done, unblock the signal handler.
            drop(th);
        }

        Ok(())
    }

    /// Consumingly apply the user picked changes to a file.
    ///
    /// **Attention**: Must be consuming, repeated usage causes shifts in spans
    /// and would destroy the file structure!
    pub fn write_user_pick_changes_to_disk(
        &self,
        userpicked: interactive::UserPicked,
    ) -> Result<()> {
        if userpicked.total_count() > 0 {
            debug!("Writing changes back to disk");
            for (origin, bandaids) in userpicked.bandaids.into_iter() {
                self.write_changes_to_disk(origin, bandaids.into_iter())?;
            }
        } else {
            debug!("No band aids to apply");
        }
        Ok(())
    }
    /// Run the requested action.
    pub async fn run(self, documents: Documentation, config: Config) -> Result<Finish> {
        let fin = match self {
            Self::ListFiles => self.run_list_files(documents, &config).await?,
            Self::Reflow => self.run_reflow(documents, config).await?,
            Self::Check => self.run_check(documents, config).await?,
            Self::Fix => self.run_fix_interactive(documents, config).await?,
            Self::Config | Self::Version | Self::Help => {
                unreachable!("qed")
            }
        };
        Ok(fin)
    }

    /// Run the requested action.
    async fn run_list_files(self, documents: Documentation, _config: &Config) -> Result<Finish> {
        for (origin, _chunks) in documents.iter() {
            println!("{}", origin.as_path().display())
        }
        Ok(Finish::Success)
    }

    /// Run the requested action _interactively_, waiting for user input.
    async fn run_fix_interactive(self, documents: Documentation, config: Config) -> Result<Finish> {
        let n_cpus = num_cpus::get();

        let checkers = Checkers::new(config)?;

        let n = documents.entry_count();
        log::debug!("Running checkers on all documents {}", n);
        let mut pick_stream = stream::iter(documents.iter().enumerate())
            .map(|(mut idx, (origin, chunks))| {
                // align the debug output with the user output
                idx += 1;
                log::trace!("Running checkers on {}/{},{:?}", idx, n, &origin);
                let suggestions = checkers.check(origin, &chunks[..]);
                async move { Ok::<_, color_eyre::eyre::Report>((idx, origin, suggestions?)) }
            })
            .buffered(n_cpus)
            .fuse();

        let mut collected_picks = UserPicked::default();
        while let Some(result) = pick_stream.next().await {
            match result {
                Ok((idx, origin, suggestions)) => {
                    let (picked, user_sel) =
                        interactive::UserPicked::select_interactive(origin.clone(), suggestions)?;

                    match user_sel {
                        UserSelection::Quit => break,
                        UserSelection::Abort => return Ok(Finish::Abort),
                        UserSelection::Nop if !picked.is_empty() => {
                            log::debug!(
                                "User picked patches to be applied for {}/{},{:?}",
                                idx,
                                n,
                                &origin
                            );
                            collected_picks.extend(picked);
                        }
                        UserSelection::Nop => {
                            log::debug!("Nothing to do for {}/{},{:?}", idx, n, &origin);
                        }
                        _ => unreachable!(
                            "All other variants are only internal to `select_interactive`. qed"
                        ),
                    }
                }
                Err(e) => Err(e)?,
            }
        }
        let total = collected_picks.total_count();
        // clustering per file is not reasonable
        // since user abort (`<CTRL>-C` or `q`) should not
        // leave any residue on disk.
        self.write_user_pick_changes_to_disk(collected_picks)?;

        Ok(Finish::MistakeCount(total))
    }

    /// Run the requested action.
    async fn run_check(self, documents: Documentation, config: Config) -> Result<Finish> {
        let n_cpus = num_cpus::get();

        let checkers = Checkers::new(config)?;

        // TODO per file clustering might make sense here
        let mistakes_count = stream::iter(documents.iter().enumerate())
            .map(move |(idx, (origin, chunks))| {
                let suggestions = checkers.check(origin, &chunks[..]);
                async move { Ok::<_, color_eyre::eyre::Report>((idx, origin, suggestions?)) }
            })
            .buffered(n_cpus)
            .try_fold(0_usize, |acc, (_idx, origin, suggestions)| async move {
                let n = suggestions.len();
                let path = origin.as_path();
                if n == 0 {
                    info!("✅ {}", path.display());
                } else {
                    info!("❌ {} : {}", path.display(), n);
                }
                for suggestion in suggestions {
                    println!("{}", suggestion);
                }
                Ok::<_, color_eyre::eyre::Report>(acc + n)
            })
            .await?;
        if mistakes_count > 0 {
            Ok(Finish::MistakeCount(mistakes_count))
        } else {
            Ok(Finish::Success)
        }
    }

    /// Run the requested action.
    async fn run_reflow(self, documents: Documentation, config: Config) -> Result<Finish> {
        let reflow_config = config.reflow.clone().unwrap_or_default();
        let reflow = Reflow::new(reflow_config)?;

        let _ = documents
            .into_par_iter()
            .map(|(origin, chunks)| {
                let mut picked = UserPicked::default();
                let suggestions = reflow.check(&origin, &chunks[..])?;
                for suggestion in suggestions {
                    let bandaids = suggestion.replacements.first().map(|replacement| {
                        let bandaid =
                            super::BandAid::from((replacement.to_owned(), &suggestion.span));
                        bandaid
                    });

                    picked.add_bandaids(&origin, bandaids);
                }
                Ok::<_, color_eyre::eyre::Report>(picked)
            })
            .try_for_each(move |picked| {
                self.write_user_pick_changes_to_disk(picked?)?;
                Ok::<_, color_eyre::eyre::Report>(())
            })?;

        Ok(Finish::Success)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryInto;

    macro_rules! verify_correction {
        ($text:literal, $bandaids:expr, $expected:literal) => {
            let mut sink: Vec<u8> = Vec::with_capacity(1024);

            apply_patches(
                $bandaids.into_iter().map(|bandaid| Patch::from(bandaid)),
                $text,
                &mut sink,
            )
            .expect("Line correction must work in unit test!");

            assert_eq!(String::from_utf8_lossy(sink.as_slice()), $expected);
        };
    }

    #[test]
    fn patch_full() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let patches = vec![
            Patch::Replace {
                replace_span: Span {
                    start: LineColumn { line: 1, column: 6 },
                    end: LineColumn {
                        line: 2,
                        column: 12,
                    },
                },
                replacement: "& Omega".to_owned(),
            },
            Patch::Insert {
                insert_at: LineColumn { line: 3, column: 0 },
                content: "Icecream truck".to_owned(),
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
    fn patch_replace_1() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();
        let bandaids = vec![Patch::Replace {
            replace_span: (1_usize, 0..1).try_into().unwrap(),
            replacement: "Y".to_owned(),
        }];
        verify_correction!("T🐠🐠U", bandaids, "Y🐠🐠U");
    }

    #[test]
    fn patch_replace_2() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();
        let bandaids = vec![Patch::Replace {
            replace_span: (1_usize, 1..3).try_into().unwrap(),
            replacement: "Y".to_owned(),
        }];
        verify_correction!("T🐠🐠U", bandaids, "TYU");
    }

    #[test]
    fn patch_replace_3() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();
        let bandaids = vec![Patch::Replace {
            replace_span: (1_usize, 3..4).try_into().unwrap(),
            replacement: "Y".to_owned(),
        }];
        verify_correction!("T🐠🐠U", bandaids, "T🐠🐠Y");
    }

    #[test]
    fn patch_injection_1() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let patches = vec![Patch::Insert {
            insert_at: LineColumn {
                line: 1_usize,
                column: 0,
            },
            content: "Q".to_owned(),
        }];
        verify_correction!("A🐢C", patches, "QA🐢C");
    }

    #[test]
    fn patch_injection_2() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let patches = vec![Patch::Insert {
            insert_at: LineColumn {
                line: 1_usize,
                column: 2,
            },
            content: "Q".to_owned(),
        }];
        verify_correction!("A🐢C", patches, "A🐢QC");
    }
    #[test]
    fn patch_injection_3() {
        let _ = env_logger::Builder::new()
            .filter_level(log::LevelFilter::Trace)
            .is_test(true)
            .try_init();

        let patches = vec![Patch::Insert {
            insert_at: LineColumn {
                line: 1_usize,
                column: 3,
            },
            content: "Q".to_owned(),
        }];
        verify_correction!("A🐢C", patches, "A🐢CQ");
    }
}
