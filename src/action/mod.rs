//! Covers all user triggered actions (except for signals).

use super::*;
use crate::checker::Checkers;
use crate::errors::*;
use crate::reflow::Reflow;

use fs_err as fs;

use rayon::iter::ParallelIterator;

use crate::paths::OutputPaths;
use std::io::{Read, Write};
use std::path::PathBuf;

pub mod bandaid;
pub mod interactive;
mod learning;
mod update;

#[cfg(test)]
mod tests;

pub(crate) use bandaid::*;

use interactive::{DocumentOutcome, UserPicked};
use learning::SessionResolutions;

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
        matches!(*self, Self::MistakeCount(n) if n > 0)
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
pub(crate) fn apply_patches<II, I>(
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
        sink.write_all(data.as_bytes())?;
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
                Patch::Insert { insert_at, content } => (*insert_at, content.as_str(), true),
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
                            source_buffer[cc_end_byte_offset..cc_start_byte_offset].escape_debug()
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
                Patch::Insert { insert_at, .. } => *insert_at,
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
                        source_buffer[cc_start_byte_offset..cc_end_byte_offset].escape_debug()
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
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Action {
    /// Only show errors
    Check,

    /// Interactively choose from checker provided suggestions.
    Fix,

    /// Reflow doc comments, so they adhere to a given maximum column width.
    Reflow,

    /// List all files in depth first sorted order in which they would be
    /// checked.
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
            ContentOrigin::TestEntityRust => unreachable!("Use a proper file"),
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
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".cargo-spellcheck.lock");
        let lock_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(std::path::Path::new(&lock_path))
            .wrap_err("Could not open the source sidecar lock")?;
        // Rename replaces the source inode, so coordinate through a stable
        // sidecar and leave it in place for other sessions waiting on it.
        lock_file
            .file()
            .lock()
            .wrap_err("Could not lock the source file")?;
        log::trace!("Attempting to open {} as read", path.display());
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
                bandaids.into_iter().map(Patch::from),
                content.as_str(), // FIXME for efficiency, correct_lines should integrate with `BufRead` instead of a `String` buffer
                &mut writer,
            )?;

            writer.flush()?;
            // Required for windows support, which does not allow
            // to move a file while it is opened, see
            // <https://github.com/drahnr/cargo-spellcheck/issues/251>
            drop(writer);
            drop(reader);
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
            log::debug!("Writing changes back to disk");
            for (origin, bandaids) in userpicked.bandaids.into_iter() {
                self.write_changes_to_disk(origin, bandaids)?;
            }
        } else {
            log::debug!("No band aids to apply");
        }
        Ok(())
    }
    /// Run the requested action.
    pub async fn run(self, documents: Documentation, config: Config) -> Result<Finish> {
        let output_paths = OutputPaths::new(&[], false)?;
        self.run_with_output_paths(documents, config, &output_paths)
            .await
    }

    /// Run an action with the caller's source-path display preferences.
    pub(crate) async fn run_with_output_paths(
        self,
        documents: Documentation,
        config: Config,
        output_paths: &OutputPaths,
    ) -> Result<Finish> {
        let fin = match self {
            Self::ListFiles { .. } => self.run_list_files(documents, output_paths)?,
            Self::Reflow { .. } => self.run_reflow(documents, config).await?,
            Self::Check { .. } => self.run_check(documents, config, output_paths).await?,
            Self::Fix { .. } => {
                self.run_fix_interactive(documents, config, output_paths)
                    .await?
            }
        };
        Ok(fin)
    }

    /// Run the requested action.
    fn run_list_files(
        self,
        documents: Documentation,
        output_paths: &OutputPaths,
    ) -> Result<Finish> {
        for (origin, _chunks) in documents.iter() {
            println!("{}", output_paths.path(origin.as_path()).display())
        }
        Ok(Finish::Success)
    }

    /// Run the requested action _interactively_, waiting for user input.
    async fn run_fix_interactive(
        self,
        documents: Documentation,
        config: Config,
        output_paths: &OutputPaths,
    ) -> Result<Finish> {
        self.run_fix_interactive_with(
            documents,
            config,
            |origin, suggestions, resolutions, checkers| {
                UserPicked::select_interactive(
                    origin,
                    suggestions,
                    resolutions,
                    checkers,
                    output_paths,
                )
            },
        )
    }

    fn run_fix_interactive_with(
        mut self,
        documents: Documentation,
        config: Config,
        mut select: impl for<'s> FnMut(
            ContentOrigin,
            Vec<Suggestion<'s>>,
            &mut SessionResolutions,
            &Checkers,
        ) -> Result<(UserPicked, DocumentOutcome)>,
    ) -> Result<Finish> {
        let n = documents.entry_count();
        let mut resolutions = SessionResolutions::new(&config)?;
        let mut checkers = Checkers::new(config)?;
        let mut collected_picks = UserPicked::default();

        'documents: for (idx, (origin, chunks)) in documents.iter().enumerate() {
            let suggestions = checkers.check(origin, chunks)?;
            log::debug!("Running checkers on {}/{},{:?}", idx + 1, n, origin);
            let (picked, outcome) =
                select(origin.clone(), suggestions, &mut resolutions, &checkers)?;
            if outcome == DocumentOutcome::Abort {
                return Ok(Finish::Abort);
            }
            collected_picks.extend(picked);
            if outcome == DocumentOutcome::Quit {
                break 'documents;
            }
        }
        let total = collected_picks.total_count();
        // Neither patches nor accepted vocabulary reach disk before completion
        // or an explicit quit-and-apply. An abort discards the whole session.
        resolutions.commit(collected_picks, &mut self, &mut checkers)?;

        Ok(Finish::MistakeCount(total))
    }

    /// Run the requested action.
    async fn run_check(
        self,
        documents: Documentation,
        config: Config,
        output_paths: &OutputPaths,
    ) -> Result<Finish> {
        let checkers = Checkers::new(config)?;
        let num_mistakes = documents
            .into_par_iter()
            .map(|(origin, chunks)| {
                checkers.check(&origin, &chunks).map(|suggestions| {
                    let path = output_paths.path(origin.as_path());
                    let n = suggestions.len();
                    match suggestions.is_empty() {
                        true => log::info!("✅ {}", path.display()),
                        false => log::info!("❌ {} : {}", path.display(), n),
                    };
                    for suggestion in suggestions {
                        let path = output_paths.path(suggestion.origin.as_path());
                        println!("{}", suggestion.display_with_path(&path));
                    }
                    n
                })
            })
            .try_fold_with(0, |count, res| res.map(|it| it + count))
            .try_reduce(|| 0, |l, r| Ok(l + r))?;

        if num_mistakes > 0 {
            Ok(Finish::MistakeCount(num_mistakes))
        } else {
            Ok(Finish::Success)
        }
    }

    /// Run the requested action.
    async fn run_reflow(self, documents: Documentation, config: Config) -> Result<Finish> {
        let reflow_config = config.reflow.clone().unwrap_or_default();
        let reflow = Reflow::new(reflow_config)?;

        documents
            .into_par_iter()
            .map(|(origin, chunks)| {
                let mut picked = UserPicked::default();
                let suggestions = reflow.check(&origin, &chunks[..])?;
                for suggestion in suggestions {
                    let bandaids = suggestion.replacements.first().map(|replacement| {
                        super::BandAid::from((replacement.to_owned(), &suggestion.span))
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
