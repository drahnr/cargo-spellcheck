# Changelog
All notable changes to this project will be documented in this file.

## [0.10.0-alpha.1] - 2022-01-21

### Bug Fixes

- Avoid mismatch, content is really just the description
- Add temporary binding, add trailing space to test
- Extend and disable description handling for now, it needs some more work

### Documentation

- Invalid "Remedies" link in README (#245)

### Features

- Write files atomically and delay signals (#224)

### Miscellaneous Tasks

- Remove redundant `'static` lifetimes for `const C: &str` (#244)
- Bump pulldown-cmark and isolang (#246)

### Testing

- Manifest demo description multiline example

### Ci

- Test dev comments too
- Don't pull submodules for test repos

## [0.9.6] - 2021-12-15

### Features

- Add deny dbg macro

### Quirk

- Disable nlprules by default (#243)

## [0.9.4] - 2021-11-24

### Features

- Add spellchecking to description field in Cargo.toml (#236)

## [0.9.3] - 2021-11-11

### Bug Fixes

- Add missing ticked original check
- Fix a few more edge cases
- Assure we use the threadpool based executor

### Documentation

- Move information out of README.md

### Miscellaneous Tasks

- Rename `klass` to `category`

## [0.9.2] - 2021-11-04

### Bug Fixes

- Lost dbg statement

## [0.9.1] - 2021-11-04

### Bug Fixes

- Toml generation
- Only distinguish by class on clustering
- Add quirk to tokenization

### Features

- Add test cases for corner cases

### Miscellaneous Tasks

- Add enum variant comments

## [0.9.0-rc1] - 2021-10-20

### Bug Fixes

- Avoid duplicate usage of same temporary file, when applying corrections
- Assure backtrace is built optimized in debug mode
- Backtick
- Quit != abort, improve user pick handling
- Avoid collecting files repeatedly

### Miscellaneous Tasks

- Bump dependencies
- Reflow
- Cleanup

### Refactor

- Allow async for latency sensitive commands (#205)
- Improve selection ui, simplify, prep for `-enclosed option

## [0.8.14] - 2021-10-13

### Miscellaneous Tasks

- Update Cargo.lock

## [0.8.13] - 2021-08-19

### Bug Fixes

- Check more places for configs.

## [0.8.12] - 2021-08-10

### Bug Fixes

- Spellcheck key is optional

## [0.8.11] - 2021-07-26

### Bug Fixes

- Improve logic around ticks
- On short texts, assure backlog is flushed.
- Avoid canonicalize

### Documentation

- Add remedy.md
- Better sample

### Features

- Show extracts when comparing spans

### Refactor

- Improved config file loading logic

## [0.8.9] - 2021-07-13

### Bug Fixes

- Reduce md lib msg from warn to debug
- Allow `ink!'s` & `should've` to work

### Features

- Add list-files subcommand
- Go depth first

### Miscellaneous Tasks

- Print stats of files check with info level
- Improve fmt

## [0.8.8] - 2021-07-09

### Workaround

- Assure the tokenizer recognizes i.e. isn't as one token

## [0.8.7] - 2021-07-08

### Bug Fixes

- If recurse, collect flat files, otherwise traverse dirs

### Documentation

- Add minimum supported rust version

## [0.8.6] - 2021-06-27

### Miscellaneous Tasks

- Remove dbg! statement

## [0.8.5] - 2021-06-27

### Bug Fixes

- Do not require a pound symbol for a raw str comment.

### Miscellaneous Tasks

- Bump ra_syn, fancy-regex and update lock

### Testing

- Assure comment variant detection stays fine

### Revert

- Undo pulldown cmark, so a release can be slated

## [0.8.4] - 2021-05-31

### Bug Fixes

- Languagetool was replaced by nlprule a while ago

### Documentation

- Clarify README

### Miscellaneous Tasks

- Ref gh issue
- Remove language tool reference from README, link to nlprule repo
- Remove 0.6.3 artifacts
- Cargo update

### Refactor

- Extract verbosity into fn

### Workaround/manifest

- Assure manifest lib path is filled with default, closes #176

## [0.8.3] - 2021-04-26

### Miscellaneous Tasks

- Avoid repeated warnings for missing terminal size
- Bump nlprule, ra_ap_syntax

## [0.8.2] - 2021-04-19

### Features

- Migrate to eyre

## [0.8.1] - 2021-04-19

### Bug Fixes

- Update README and remove languagetool references, add config options

### Miscellaneous Tasks

- Bump ra_ap_syntax

## [0.8.0] - 2021-04-18

### Bug Fixes

- Allow , separate list of checkers as flag arg
- Lazy_static does not play nice with rust analyzer F2

### Features

- Allow vulgar fractions and emojis
- Use typed language tag
- Trigger on webhook, rather than polling only

### Miscellaneous Tasks

- Add some emoji chars
- Bump nlprule to 0.6.3

### Workaround

- Move to git version temporarily

## [0.8.0-beta.3] - 2021-04-16

### Bug Fixes

- Assure deps are ok, add note about --locked
- Do not exit early on empty cache
- Add hunspell data

### Features

- Include builtin fallback dictionary and affix files
- Some more emojis

### Miscellaneous Tasks

- Bump nlprule to 0.6.2 to get the speed gain

## [0.8.0-beta.2] - 2021-04-16

### Bug Fixes

- Handle globs
- Avoid nlprule smart quotes
- Properly use globs
- Demo workspace references a README which was missing

### Miscellaneous Tasks

- Bump cargo_toml to 0.9
- Bump nlprule to 0.6.1
- Add some emojis to trace level logging

## [0.8.0-beta.1] - 2021-04-11

### Bug Fixes

- Add backtrace info
- Inline code shows up in the plain overlay, but not in the span list
- Length limit must be accounted for
- Additional file must be covered in recurse
- Cleanup artifacts

### Features

- Lay groundwork for inline code block handling
- Limit to aliases to 16 chars

### Miscellaneous Tasks

- Avoid bails when ci is under high load
- Bump to 0.5.3
- Update deps, add debug info to release builds
- Bump nlprule to 0.6.0
- Update deps, add debug info to release builds
- Bump ra_ap_syntax
- Minor cleanups
- Bytes = chars case is always upheld
- Sigh

## [0.8.0-alpha.0] - 2021-03-18

### Bug Fixes

- Cover /*, /**, and /*! (#156)
- Cover /*, /**, and /*!
- Additional tests and logic for block comments
- Rebase fallout
- Adjust byte offset for new tokenizer
- Ignore known sentence ctrl characters
- Make sure all modules are found

### Features

- Add --jobs option and improve a few bug on messages

### Miscellaneous Tasks

- Bump deps bitflags + ra
- Cargo update
- Cargo format use fs_err where possible

### Refactor

- Log messages about thread count / jobs
- Add logs

### Ci

- Make timeouts more generous to not fail builds on initial runs

## [0.7.1] - 2021-02-24

### Bug Fixes

- Avoid re-linking
- Extra dictionary must start with a number

### Features

- More dictionary formt test cases

### Miscellaneous Tasks

- Fmt

### Refactor

- Reduce duplicate compilations in the CI pipeline

### Testing

- Add a integrity test of the hunspell extra dictionary logic
- Add tests for dictionary validation

## [0.7.0] - 2021-02-23

### Bug Fixes

- Use nlprule config where it should be
- Allow parallel ci verification runs
- Mask disabled checkers based on checkers

### Documentation

- Remove obsolete dev comment
- Better doc comment

### Miscellaneous Tasks

- Warnings
- Make very nested deps quiet independently of -v flags

### Refactor

- Make args part of config module
- Re-partition config modules
- Introduce UnifiedArgs to consolidate Arg/Config variables

### Testing

- Better unification tests

## [0.7.0-beta.4] - 2021-02-22

### Miscellaneous Tasks

- Cargo include was wrong, extra hoops unnecessary

## [0.7.0-beta.3] - 2021-02-21

### Bug Fixes

- Include
- Pulish does not like files in the git tree being modified
- **.suffix is not ok, must be **/*.suffix

### Features

- Include bin artifacts, bump to 0.4.6 for transform support

## [0.7.0-beta.2] - 2021-02-19

### Miscellaneous Tasks

- Cargo update

## [0.7.0-beta.1] - 2021-02-14

### Bug Fixes

- Micro opt
- Avoid sorting subsets without need
- Avoid punctuation, shorten ^^^ length by one
- Remove srx, it's not configurable but part of the tokenizer
- Use the correct override paths
- Misleading expect message
- Accidentally added duplicate .bin files

### Features

- Properly sort a suggestion set
- Spit into sentences before passing to nlprule crate
- Use local files on building if available
- Nlprule uses srx internally to split into segments
- Use nlprule-build in build.rs
- Default for checkers
- Use nlprule-data as cache dir

### Miscellaneous Tasks

- Fmt
- Verify packaged file correctness after download in build.rs
- Add nlprule data
- Format
- Nlprules are language tool extracts, hence LGPLv2.1
- Use nlprule release
- Keep compressed archives instead of uncompressed
- LGPLv2.1 is not relevant for the source, only for the binary
- Remove old way of obtaining binaries
- Make logs visible
- Fmt
- Rename check_sentence -> check_chunk
- Detail README
- Reflow README.md
- Remove nlprule-data due to size limit of crates.io
- Avoid caching for now, break cargo publish

### Refactor

- Remove duplicate regex serialization code, add config for nlp
- Reduce checker invocation repetitity

## [0.6.2] - 2021-02-01

### Bug Fixes

- Add missing import for reflow tests
- Quit should apply all picked changes so far

### Miscellaneous Tasks

- Bump dependencies
- Fmt+fix

## [0.6.1] - 2021-01-22

### Bug Fixes

- Add version flag action extraction, add test
- Parse plain one last, order matters, adjust executable name sanitization code

### Documentation

- Add section on git hook usage

### Miscellaneous Tasks

- Silence an import warning about a debug only import
- Fmt + fix

## [0.6.0] - 2021-01-18

### Miscellaneous Tasks

- Fix issue template typo
- Enlist dev comment feature

## [0.5.0] - 2020-11-15

### Bug Fixes

- Adjust to added link fragment count
- Iterator must not drop the last line
- Iterator should not drop the last char
- Test adjustments
- Tokeneer tests and iterator behaviour adjustments
- First impl of reflow_inner() and reflow()
- Use CommentVariant to get proper prefix
- Remove old fix for non-existent bug
- Fixed bug in markdown parsing
- Return none if no reflow occurred
- Remove unused argument
- Split sets if last line is empty, fix tests accordingly
- Use is_empty()
- Span calculation was bad
- Don't split set on empty line
- Only operate on relevant part of chunk
- Make unbreakable ranges absolute, clear stack after storing
- Use original string to check if reflow occurred
- Add /// or //! for newlines which appeared in reflow
- Added missing clear() of unbreakable_stack
- Check if reflow happend was bad
- Use correct indentations on all comment variants
- Iterator lengths did not match
- 'proof-of-concept' for multiline bandaids
- Tried to fix multiline bandaids
- Introduce FirstAidKit for multiline bandaids
- Removed old multiline bandaid approach
- Fix FirstAidKit::try_from() to work on correct spans
- Use char count instead of byte len for span length
- Exchange iterators
- Use try_from(suggestion) for multiline spans
- Fix finding length of lines
- Use bandaid enum for better insertion/deletion of lines
- Insertion is placed before specified line
- Add method to calculate line length
- Correct reflow of macro doc comments
- Add proper pre- and suffix
- Ranges must be in chars
- Correct reflow of macro doc comments
- Add proper pre- and suffix
- Fix tests and remove unused stuff
- Hash-doc-macro comments are reflown properly
- Improve doccomments
- Index operator of String is in bytes(!) [], use util::sub_chars
- Use saturating_sub
- Start to fix markdown
- Make sure the right span bounds are used
- Fmt
- Fix line feed
- Remove outdated hack
- Also check `find_spans` equiv with `find_coverage` for specific spans
- Sub_char_range should return empty string for oob conditions
- Be explicit about magic expect numbers
- Only set the end if we terminate iterations
- Assure consistency in prefix and suffix
- Impl a hack to deal with different indentation depths
- Simplify indentation test one more time
- Correct indentation for /// and //!
- Do not delete first space
- Do not ask the user, just apply the replacement
- Ensure reflow binary is executable
- Make sure all ranges are in chars
- Assure the collected unbreakable byte ranges are used
- Remove dbg! statement
- First attempt at adding the consumed newline
- Cmark reflow now works beside a minor newline issue
- Do not introduce newlines when content was empty
- Assure rw for git dir in validate-reflow
- Assure target dir is also cached between runs

### Documentation

- Check reflow feature
- Improve doc comments
- Adjust README to how it should be
- Resolve TODO

### Features

- Check content origins in parallel
- First shot at rewrapping
- Initial impl outline w/ cmark awareness
- Add indentation awareness to the iterators
- Introduce comment variant type to literal
- Hook up args
- Added comment variant for //!
- Improve error msg
- Use 🐠 and 🐢 to check if non-ascii chars work too
- Improve error msg
- Parallelize reflow per file
- Extract newline instead of assuming \n
- Add allocation free version of sub_chars as sub_char_range
- Sub_char_range works with all kinds of ranges now
- Reduce amount of data in reflow tests
- Add test and document config items
- Set thread num for debug to 1
- Test reflow in PRs too
- Add another test, print replacmenets of suggestions too

### Miscellaneous Tasks

- Wrapper -> reflow, markdown -> cmark
- Update settings.json
- Format
- Some clippy warnings
- Documentation and some clippy warnings
- Set reflow default limit to 80
- Formatting
- Added and reviewed documentation
- Added TODOs
- Better todos
- Test fallout due to rebase
- Disable invalid test assumptions
- Formatting
- Fix blown-up rebase
- Rebase fixes
- Add review suggestions
- Add a warning if we fallback
- Minor
- Make sure env logger is enabled
- Miniscule
- Simplify `find_spans` in `Chunk`
- `correct_lines` -> `apply_patches`
- Use more generics
- Cleanup
- Cleanup debug statements, cargo fix
- Review nitpicks
- Added dots to comments
- Add additional debug output
- Rename task
- Remove excessive logging
- Cleanup comments
- Run reflow on README.md
- Remove comment
- Avoid another string copy
- Remove stray dbg!
- Remove commented code, dbg!

### Refactor

- Split iterator into two
- Avoid collect for literal set iterator over literals
- Rename find_spans_inclusive to find_enclosing_spans()
- Adjust to code style
- Return iterator on find_covered_spans
- Remove unused code (and fix tests)
- Extract `store` closure into own function
- Split `FirstAidKit` from `Bandaid` file
- Avoid mix of concerns of suggestion,pick idx and bandaid
- Switch to enum
- Simplify
- Use enum for replacement
- Use repeat() instead of ugly vector join()
- Use enum for replacement
- Make sure symmetry is kept
- Rewrite correct_lines purely based on spans
- Split `FirstAidKit` from `Bandaid` file
- Use enum for replacement
- Use enum for replacement
- Make sure symmetry is kept
- Avoid abusing Err for a non-error case
- Refactor the line delimiters impl
- Introduce `struct Indentation`
- Split tests into separate file
- Do not process exit if exit code would be 0
- Avoid unnecessary allocations
- Avoid crate::util::* prefixes all over the place

### Testing

- Add test for very short comments
- Correct expectations
- Add test struct with too long lines
- Add tests for doc comments
- Added a long line with indentaion
- Added better demo cases for reflowing
- Add multiline test for correct_lines()
- Added macro for correction testing
- Remove redundant replacement check
- Added demo for another reflow case
- Extend tests for bandaids to #doc
- Started to fix tests
- Add commentvariant in action mod
- Add multiline test for correct_lines()
- Add commentvariant in action mod
- Add anoher long demo coment
- Distill a coverage vs find_spans test to showcase issues
- Simplify failing test
- Check span of reflow-suggestion
- Make unit test pass
- Move tests to own file
- Cleanup reflow test macros, add README.md test macro
- Add a minimal test case
- Add expect to cmark tests, expand reflow test macros
- Allow checking against individual patches as well
- Improve test output for reflow_content! macro's patches variant
- Always print the patch index

## [0.4.6] - 2020-10-23

### Bug Fixes

- Opencollective -> open_collective
- Nitpick
- Resolve the config with the cwd before falling back to user default

### Features

- Use 🐠 and 🐢 to check if non-ascii chars work too
- Be more forgiving and extend the lingua dictionary

### Miscellaneous Tasks

- Update dependencies and address fallout
- Add opencollective
- Print config already on debug instead of trace

### Refactor

- Rewrite correct_lines purely based on spans
- Simplify by using serde(default) annotations

## [0.4.5] - 2020-09-11

### Bug Fixes

- Improve borked test, rename ambiguous names
- Add emoji test to common mark tests, use chars instead of byte ranges
- Adjust `fn find_spans()` to include partially overlapping spans
- Reduce test coverage overlap, improve non overlap coverage
- ${project}/.config/spellcheck.toml was not resolved, dictionaries misspelled
- Upstream bug with tables, circumvention
- Insert additional spaces after list items and at list start
- Ignore tables for now
- Make sure replace the table by exactly one empty line

### Documentation

- Frenzy, complete full docs coverage

### Features

- Introduce lints

### Miscellaneous Tasks

- Remove unused dependencies, bump some deps
- Unused use statements
- Signal crate must be used even on windows
- Adjust bug template
- Feature template
- Question template
- Assure log level is always trace, add e2e test module
- Remove stale debug messages
- Unify BUG logs, fmt
- Warning
- Reduce and add target for find_spans
- Extra logs and improved lingo.dic
- Remove obsolete feature item

### Refactor

- Move the handling of file types into documentation
- End to end test for issue case, extract macro

## [0.4.4] - 2020-08-31

### Bug Fixes

- Improve syntax and variable names
- Add some changes in the examples
- Introduce bad reference link handler
- Links shall work

### Documentation

- Improve doc comments

### Miscellaneous Tasks

- .config/spellcheck.{conf,toml}
- <at>todo -> TODO
- Bump cargo lock
- Cleanup
- Un-ignore test cases, cleanup warnings and TODOs
- Silence unused warnings
- Remove dbg!

### Refactor

- Re-work cmark tests a bit

### Testing

- Add parametrized tests for linktypes

### Autolink/markdown

- Handle auto links to skip suggestions

### Check/markdown

- Check all possibles LinkType's supported

### Email/markdown

- Handle email links to skip suggestions

### Footnote/markdown

- Handling footnote to be continued

## [0.4.3] - 2020-08-22

### Bug Fixes

- Glob asterix must be outside quotation
- Handle extension delta for artifacts better
- Properly handle extra dictionaries lookup
- Adjust the erased lines to show the question

### Documentation

- Be more precise about the wording
- Add references on how to define custom dictionary files

### Miscellaneous Tasks

- Add gitignore for demo

## [0.4.2] - 2020-08-20

### Bug Fixes

- Revert release task
- Signal_hooks is not working on windows

### Features

- Prepare windows releases

## [0.4.1] - 2020-08-20

### Bug Fixes

- Default dirs should always be kept
- Capacity is in bytes, not chars
- Allow concatenation was inverted, added counterpart + tests

### Features

- Add rust-lang/rust and spearow/juice as test cases for PRs

### Miscellaneous Tasks

- Improve issue template
- Fallout

### Styling

- No suggestion, less whitespaces

### Testing

- Add test no suggestion, no spaces

### Src/suggestion

- Improve check

## [0.4.0] - 2020-08-17

### Documentation

- Add quirks documentation

## [0.4.0-alpha.3] - 2020-08-14

### Bug Fixes

- Do not ignore the error value of individual checkers

## [0.4.0-alpha.2] - 2020-08-14

### Bug Fixes

- Better error messages on missing config

### Features

- Add whitelist to config
- Implement transformation
- Recursive transformations and whitelisting
- Extract quirks from hunspell, add allow_concatenation

### Miscellaneous Tasks

- Vscode

### Refactor

- Partition into more fns, add tests, add samples

### Review

- Precision comments
- Full sentence prefered

## [0.3.1] - 2020-08-04

### Bug Fixes

- Covered lines extraction

### Refactor

- Covered lines search does not require peekable

## [0.3.0] - 2020-07-29

### Bug Fixes

- Always recurse manifests, improve hunspell logs
- Only use fix for interactive fixes

### Documentation

- Re-word

### Miscellaneous Tasks

- Re-word cargo toml file system completion

### Testing

- Adjust to new behaviour

### Cargo

- Fmt + fix

## [0.3.0-beta.5] - 2020-07-28

### Bug Fixes

- Txt[..] must be done with sub_chars(..)

### Features

- Better debug traces

## [0.3.0-beta.4] - 2020-07-28

### Bug Fixes

- Cargo_toml does some weird completions which do not exists

### Miscellaneous Tasks

- Duplicate use statement for util::sub_chars

## [0.3.0-beta.3] - 2020-07-28

### Miscellaneous Tasks

- Remove dbg! statements

## [0.3.0-beta.2] - 2020-07-28

### Features

- Add test for unicode chars

## [0.3.0-beta.1] - 2020-07-27

### Bug Fixes

- Change logic for fitting long statements
- Use iteractor and add new logic
- Use name exercpt instead of padding_till_literal
- Clean up and remove verbosity
- Rebase fallout
- Add a space to test str
- Constrain terminal size to 80 chars for tests
- Avoid showing prefix/suffix ... when nothing was cut
- Rework the offset calculation for the relevant line
- Unused vars
- Extraction by span was off by one
- Missing manifest items shall not lead to an abort
- Remove obsolete comments
- Extract sub span sanitization and improve test coverage
- There are generated docstrings which could contain "
- Workspaces and incomplete or missing files should be forgiven
- Use fancy-regex with backref for better #[doc=".. detection
- Prevent dots in cases of full left/right context printing
- Dots are not additional

### Documentation

- Tick the right boxes

### Features

- Call truncate method to displayed string
- Do not abort due to issues with individual files
- --skip-readme ignores manifest description and readme file

### Miscellaneous Tasks

- More literal tests, less warnings
- Accuracy of underline
- Rustc pair programming
- Remove obsolete fn

### Refactor

- Make code more idiomatic rust
- Hide the stream and allow access to content
- Generalize and test logic, add util, review comments literal

### Chunk

- Add len_in_chars()

### Clean-up/suggestion

- Clean up args

### Clean-up/suggestion.rs

- Fix comments and remove not needed code

### Cleaup/chunk

- Remove char_sub_window

### Src/suggestion

- Add dots in the comment lines

## [0.3.0-alpha.6] - 2020-07-23

### Bug Fixes

- Add allow unused in a few spots
- Allow common mark input
- Better test macros and helpers
- More test infra fixes for multiline handling
- Find_spans again
- Avoid dbg!(suggestion) and a few slips of pens
- Single character tokens must also have a span length of 1
- Assure the correct range is asserted
- Span, chunk, and tests are now multiline ready
- Warn -> trace
- Exit code must be zero now unless there was an error in the program
- Failed span calc for indented lines
- Adjust spans properly
- Improve the detection & span adjustments of /// comments
- If no indentation str is given, use ""
- Extract the spans properly

### Documentation

- Add link to multiline replacements to issue #25
- Refactor
- Fragment explanation
- Dev comment

### Features

- Initial handling of multiline literals
- Verify suggestion is dbg printable
- Allow prefix syntax for chyrp_up with @

### Miscellaneous Tasks

- Remove commented dbg! helpers
- Use vs cargo fix

### Refactor

- Raw mode guard drop code simplification
- Find_spans
- Err(anyhow!(...)) becomes bail!(...)
- Err(anyhow!(...)) becomes bail!(...)

## [0.3.0-alpha.5] - 2020-07-10

### Bug Fixes

- Bump hunspell to v0.3.0, assure hunspell is bundled

## [0.3.0-alpha.4] - 2020-07-10

### Bug Fixes

- Update lock file

### Documentation

- Fix installation instruction for macOS with hunspell support
- Remove note about windows issue

### Miscellaneous Tasks

- Update to v0.2.0

## [0.3.0-alpha.3] - 2020-07-09

### Bug Fixes

- When enabling trace, plain overlay display could cause a panic
- Use correct expect for EXPECTED in suggestion fmt test
- Format display temprory solution until #65 lands

### Documentation

- Update README.md about windows troubles and cleanup

### Features

- Return user selection discrimnator
- Properly propagate signals from raw mode back to main
- Add a suggestion format test
- Return the suggestion sets for end2end tests for further checks
- Track range to avoid duplicate computations

## [0.3.0-alpha.2] - 2020-07-07

### Bug Fixes

- Assure PRs are tested for usage with pipes
- Use grep instead of rg
- Prefer cargo run over explicit paths
- Simplified signal handling
- Print suggestions to stdout instead of stderr
- Exit right in interactive on ctrl+c
- Remove target_os cfg scopes
- Always exit with 130

### Refactor

- Remove duplicated code for terminal restoring

## [0.3.0-alpha.1] - 2020-07-03

### Bug Fixes

- Prefix and suffix trimming now uses regex and handles more variants
- Less @todo why???s
- Make tests compile quickly
- Bail if line is 0, 1-indexed
- Ignore some, make helpers public
- Gen_literal_set did not care about source passed to it
- Adjust ranges, add fluff_up! macro
- Remove zero length ranges and spans in find_spans
- Raw_extract_0 + _1
- Span calculation
- Missing 3 slashes offset
- Set_into_chunk for literalset
- R".." is a valid variant
- Span to range conversion requires the chunk
- Use chunk.as_str() for range, which is the correct reference
- Remove dbg! statements, allow x.len() == range.end as good case
- Find_spans was picking way too many items
- Introduce a magic number to make tests work
- Test multiline find_spans
- Span::try_into -> Range is not valid in this context
- Prep for the last remaining bug
- PlainOverlay::find_spans now passes too
- Remove overly verbose prints, leftover `dbg!(..)`s
- Minor code cleanups
- Use debug_assert
- Restore terminal mode on SIGTERM, SIGINT and others

### Documentation

- Add samples
- Add module docs

### Features

- Addition test checker fns for span extraction of original content
- ChunkDisplay impl
- Add fluff_up tests
- Recover range from span and collected content
- Impl Display

### Miscellaneous Tasks

- Minor changes
- Rename test cases and remove obsolete @todo entries
- Split unit-tests from compile check

### Refactor

- Introduce unit tests for module traversal
- Split chunk and cluster from documentation
- Move test and expand literal with trimmed checks
- Remove quirk, additional tests + X
- Additional line wraps
- Improve tests to use const items for expected and input vars
- End2end needed some modularity
- Avoid proc_macro::Span and use custom Span, remove TrimmedLiteralRef

### Testing

- Added simple tests
- Improved tests
- Fixed test macro
- Add test for literalset into chunk
- Clean up, use test body from checker/mod.rs

## [0.2.4] - 2020-06-19

### Bug Fixes

- Workspace and bin/lib are not mutually exclusive

### Miscellaneous Tasks

- Funding yaml
- Adjust issue templates
- Simplify PR template

## [0.2.3] - 2020-06-18

### Bug Fixes

- Do not make PR checks sequential
- No empty links
- Update interactive example
- Handle virtual workspaces, track lib.rs entry points

### Documentation

- Link to issues in the roadmap
- Link to issues in the roadmap, with proper links
- Better headlines

### Miscellaneous Tasks

- Add license files

### Refactor

- Introduce constants and remove magic literals

## [0.2.2] - 2020-06-17

### Features

- Add insert mode

### Miscellaneous Tasks

- Assure no PRs are skipped, pull PRs first

## [0.2.1] - 2020-06-16

### Bug Fixes

- Cursor on custom user input position moves wrongly
- Remove debug message

### Miscellaneous Tasks

- Debug pane and load output dirs from cargo check
- Remove unused var

## [0.2.0] - 2020-06-16

### Bug Fixes

- Master validate checks found expected issues, but must not fail job
- Allow custom fixes

### Documentation

- Ubuntu's package has a lib prefix

### Miscellaneous Tasks

- Update Cargo.lock

## [0.1.5] - 2020-06-15

### Bug Fixes

- Do not error if search paths do not contain dic/aff files
- Only one affix file can ever be provided to hunspell
- Anotate yet-to-be-used fn as allow unused

### Features

- Test demo dir with the current revision of cargo-spellcheck

### Miscellaneous Tasks

- Issue/pr template adjustements
- Additional run test

### Refactor

- Simplify loading affixes, fix acc/dic mixup

## [0.1.4] - 2020-06-15

### Bug Fixes

- Explicitly change ownership of resource and cache dir
- Make PR checks work
- Don't advance to next suggestion on help
- Github release must stop failing
- Terminal resize while waiting user input caused crash

### Features

- Handle ctrl-c

### Miscellaneous Tasks

- Borrow and adapt issue template from concourse/concourse

## [0.1.2] - 2020-06-12

### Documentation

- Proper sample display of interactive mode

### Miscellaneous Tasks

- Trace and dbg balance

## [0.1.1] - 2020-06-12

### Bug Fixes

- Use a custom rust builder image
- Be explicit about the user and toolchain used
- Remove unused and pointless calls

### Documentation

- Update sample usage

### Miscellaneous Tasks

- Adjust colours of interactive picks

### Refactor

- Btter interactive display

## [0.1.0] - 2020-06-11

### Features

- Implement a basic replacement selection

### Testing

- Assure command line stays compatible moving forward

## [0.0.16] - 2020-06-10

### Bug Fixes

- Clarify config subcommand and add --help
- Error if extra dictionaries can not be added
- Canonicalize relative paths in relation to the config dir, instead of cwd
- Drop the raw terminal mode guard before printing help
- Only use the long option in usage
- Allow intended usage

### Documentation

- Update README.md

### Refactor

- Avoid anyhow:: all over the place
- Avoid anyhow:: all over the place

### Testing

- Also check if #{doc ="..."] works

## [0.0.15] - 2020-06-07

### Bug Fixes

- Applying the corrections was flawed
- Magic offset
- Apply needs a magic number
- Properly translate Span into Range
- Columns for start + end being equiv is not an issue
- Marker length was short one character
- Avoid warnings, includes one unreachable pattern resolution

### Documentation

- Improve a couple of doc entries

### Features

- Initial attempt writing back changes to disk
- Add tokenize test

### Miscellaneous Tasks

- Remove commented code
- Improve trace logs
- Minor
- Migrate src/tests to demo dir
- Remove dbg!

### Refactor

- Extract correct_lines fn from correction
- Rename printable to display, and add fn display(range) for convenience
- Traversel now implements an iterator

### Testing

- Add a test and add docs, adjust action test

## [0.0.14] - 2020-06-05

### Bug Fixes

- Adjust cargo toml test expectations
- As_untrimmed_str was returning a trimmed string
- Tokenization did drop a word if it ended with the string

### Documentation

- Improve README.md
- Explain why both cargo_toml and toml crates are needed
- README.md update again
- Add more inline docs to TrimmedLiteralRangePrint

### Features

- Partial impl of interactively applying fixes

### Miscellaneous Tasks

- Remove explicit tag of run in traverse module

### Refactor

- Introduce SuggestionSet and move from paths to CheckItem (WIP)
- Introduce action.rs

### Testing

- Use untrimmed in failing unit test

## [0.0.13] - 2020-06-03

### Bug Fixes

- Filter statement was inverted

### Documentation

- Update README.md
- README.md improvements

## [0.0.12] - 2020-06-03

### Bug Fixes

- Config subcommand works now
- Improve tests some more
- Use a relative offset instead of the source file span

### Features

- Span -> Range
- Initial concourse ci description

### Miscellaneous Tasks

- Launch.json
- Use more italic
- Remove dbg! macro usage

### Refactor

- Simplify argv parse with match instead of if let

### Testing

- Add better test result checks

## [0.0.11] - 2020-05-29

### Bug Fixes

- Checker filtering as cmd line arg, better tests

### Features

- Better tests

## [0.0.10] - 2020-05-28

### Bug Fixes

- Allow debugging the full application
- Additional empty suggestions
- Properly use CARGO_SPELLCHECK and -vvv

### Features

- Enable PlainOverlay usage in order to simplify
- Make tests more complicated
- Better custom debug fmt
- Better debug print

### Miscellaneous Tasks

- Remove unused imports and fn
- Test and logging cleanups

### Refactor

- Separate example
- Reduce

## [0.0.9] - 2020-05-26

### Bug Fixes

- Convert rel to abs paths, make cargo-spellcheck and cargo spellcheck work
- Additional log flags, better cmdline parsing (again)
- Module resolution deque must initialized

### Documentation

- Replace redundant line

### Features

- List entry points and follow module declarations
- Smarter recursion by extracting modules
- Add --version flag
- Add Eq + Hash impl to Spans

### Miscellaneous Tasks

- Missing `,`
- Add `all` feature
- Remove unused log::* imports

## [0.0.5] - 2020-05-20

### Documentation

- Update install and TODOs

### Miscellaneous Tasks

- 0.0.0 release
- Skeleton

<!-- generated by git-cliff -->
