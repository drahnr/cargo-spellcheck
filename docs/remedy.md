# Fixing spelling mistakes

While cargo-spellcheck is good at _pointing out_ existing spelling mistakes,
it's sometimes not obvious how to resolve them or what the correct way is
to resolve them.

The following covers an abstracted set of commonly encountered `cargo spellcheck`
complaints and how to resolve them:

## Configuration

Make sure your runs are idempotent if you run on two different systems,
which is easiest achieved by using the builtin affix and dictionaries
besides the topic specific lingo dictionary that should come with your project.

```toml
# .config/spellcheck.toml

[Hunspell]
# snip
skip_os_lookups = true
use_builtin = true
# snip
```

---

To run only spelling checks, pass `--checkers=hunspell`, `--checkers=spellbook`,
`--checkers=zet`, or a comma-separated combination of the spelling backends
available in your build. This avoids the `nlprule` grammar backend, which can
produce false positives.

## Accepting project vocabulary

Press `?` at any interactive prompt, including text editors, for context-specific
`key - brief side effect` lines ending with `? - show commands`, without headings
or prose. Help leaves input, cursor, and selection unchanged; it never applies,
confirms, learns, or cancels anything. `?` is reserved for help, not inserted as text.
Inline prompts contain only single-character keys; help explains Enter, arrows,
Esc, Ctrl-C, and each command's actual effect. The main prompt is
`Resolve this issue [y,n,q,a,d,j,e,?]?`, omitting `a` for unlearnable findings
(`Resolve this issue [y,n,q,d,j,e,?]?`).

During an interactive fix session, select the `{word} < learn` row to keep the
source unchanged and immediately see inferred affix patterns. The best
engine-validated pattern is preselected, and the unchanged plain word is initially
the last choice (or the only choice when no pattern is inferred). The table shows
`stem | suffix`, or `prefix | stem | suffix` if the dictionary's full supported
catalog contains a productive prefix, even when no inferred row has one. Flags
appear as `/U` or `/S`, and missing flags as `-`; a header aligns with the padded
value columns. `< pattern` and `< plain` hints remain outside the highlight.

At `Affixes [y,a,e,n,?]: `, Left/Right cycles backward/forward through **all
supported bundles on the current stem**, including plain, every supported suffix
or prefix, and every allowed prefix/suffix pair—not only patterns that generate
the original word. A visible horizontal strip below the table shows alternatives
as `-` or `/FLAGS`, globally alphabetically sorted by complete flag strings, plain
first: for example, `- /D /G /R /S /U /UD /US`. The initial selection remains the
best inferred match, not the first label. A `^` on the next row stays centered
under the selected label's display-width midpoint as Left/Right shifts the strip.
Both directions wrap; narrow terminals clip whole labels and show `..` at edges
with hidden choices. Strip labels and caret use the default color.

Rules marked `N` never form cross-product pairs. The selected stem stays exactly
unchanged. The same row, strip, and expansion update in place, without remounting
the prompt or an unchanged header; flagged rows show `< pattern`, unflagged rows
`< plain`.

Enter or `y` goes **directly to the full confirmation preview**, without an editor.
`e` opens `Stem [?]: ` prefilled with the selected stem and retaining its flags.
Enter validates and updates that row and its expansion, then **returns to the
affix table**, remounting prompt/header while preserving the strip's selected
flags; later cycling uses the edited stem. Esc preserves the previous row and
selection; Ctrl-C aborts. `n` or Esc in the table
cancels learning. At `Confirm learning [y,e,n,?]: `, Enter or `y` records the
entry; `n` or Esc declines, and `e` edits the stem with its flags retained and
shows a fresh full confirmation preview on Enter. Nothing is learned merely by
selecting or cycling a pattern, or accepting the editor.

Press `a` **in the affix table** for the manual suffix → stem choices → mandatory
stem editor flow, including its explicit plain-word option. At `Suffix [y,e,n,?]: `,
Enter or `y` on a suffix opens `Stem [y,e,n,?]: `; `e` instead opens the editor
for that selected suffix entry directly, skipping candidates. In the stem picker,
`e` opens the editor just like Enter or `y`. The editor uses `Stem [?]: `;
Enter shows confirmation, while Esc returns to the originating picker. Cancelling
the manual suffix picker cancels learning rather than returning to the automatic
table.

The main fix prompt's `a` shortcut still opens `Learn "{word}" [p,e,n,?]: `;
unknown custom replacements offer `Learn replacement "{word}" [p,e,n,?]: `.
Both highlight the word in bold green on black. Enter or `p` continues to inferred
affix choices, while `e` first pre-edits the word at `Word [?]: `. `n` or Esc skips
learning without discarding a pending source replacement.

Row descriptions such as `< learn`, `< custom input`, and `< wrap backticks`
align in one terminal display column within each rendered list, accounting for
checker suggestions without descriptions too. Padding and hints remain in the
default color outside the selected content's highlight. The column can move as
custom input changes; separate menus calculate their own alignment. Learning
pickers truncate visible content to a shared budget that leaves room for the
longest hint, avoiding wrapped rows. Affix tables use terminal display widths and
Unicode-safe stem clipping, falling back to a clipped `stem/FLAGS` entry when the
terminal is too narrow for columns. Help remounts the header without changing the
selection, strip, or caret position. The automatic picker reserves three context
rows below the list: alternatives strip, caret, and expansion. Manual suffix and
stem pickers retain only the expansion context row.

Both the one-line picker expansion and the full confirmation preview leave each
form's longest unchanged contiguous stem core in the default color and highlight
added or changed prefix and suffix spans in bold green on black. For example,
`hope` → `hoped` highlights `d`, `city` → `cities` highlights `ies`, and `root` →
`unrooted` highlights `un` and `ed`, leaving `root` unstyled. An unchanged base
form is unstyled. The picker context truncates to terminal width without leaking
ANSI styling, while confirmation shows the full expansion.

This works with Hunspell, Spellbook, and Zet, including Rust-only builds.
Confirmed expanded forms are immediately recognized across all enabled spelling
backends, including queued diagnostics and later files; grammar diagnostics remain
unlearnable. Custom replacements edit the source and offer optional learning for
unknown words; skipping learning does not discard the source edit.

Accepted vocabulary persists even without a `learning_dictionary` setting.
The learning target is validated at fix-session startup, before any interactive
prompt. Only a missing target (automatic or explicit) produces this warning,
exactly once, naming the chosen absolute path:

```text
Warning: learning dictionary {absolute path} does not exist; a new file will be created when learned entries are saved.
```

An existing writable regular file produces no warning. Existing nonfiles,
read-only files, and dangling symlinks fail before interaction. Learning prompts
show no persistent-learning, session-only, or missing-configuration notices.

The preferred automatic target is `.config/spellcheck/learnings.dic` under the
**current working directory**. If it is absent and `.config/learnings.dic` already
exists there, the existing legacy dictionary is used instead; if both exist, the
preferred file wins. Existing
automatic vocabulary is loaded into every active, compiled spelling backend on
fresh `check` and `fix` runs without adding it to TOML `extra_dictionaries`.

A missing dictionary (automatic or explicit) and its parent directories are
created only when a nonempty set of confirmed entries is committed on normal
completion or `q`. Abort, check-only runs, and sessions with no recorded learning
entries, including cancelled or declined learning, create no dictionary, lock
file, or empty configuration directory. cargo-spellcheck never guesses a write
target from `extra_dictionaries` and never rewrites your TOML file.

To select another dictionary explicitly, set `learning_dictionary` under an
enabled `[Hunspell]`, `[Spellbook]`, or `[ZSpell]` (Zet) section. The setting remains
per-backend; there is no top-level TOML setting. For example, for
`--checkers=hunspell` configure:

```toml
# .config/spellcheck.toml
[Hunspell]
search_dirs = ["."]
extra_dictionaries = ["lingo.dic"]
learning_dictionary = "lingo.dic"
skip_os_lookups = true
use_builtin = true
```

For `--checkers=spellbook` or `--checkers=zet`, use the same settings under
`[Spellbook]` or `[ZSpell]` instead. The learning target is relative to the
configuration file (or may be absolute). It need not exist yet: a missing target
is allowed when that same path is registered in `extra_dictionaries`, and creation
is deferred until confirmed entries are saved. An existing file must be a writable
regular UTF-8 Hunspell `.dic` dictionary; a manually created empty dictionary
contains `0` followed by a newline. The explicit target is authoritative and never
uses or creates an automatic fallback. A typo that does not match its
`extra_dictionaries` registration remains a validation error. If multiple spelling
backends are enabled, configure the target under any one of them, include that
same dictionary in **every enabled spelling backend's** `extra_dictionaries`, and use the same
`lang` for each. Multiple `learning_dictionary` settings must resolve to the same
file; conflicting targets, missing shared registrations, or differing
languages are errors.

On normal completion or `q`, accepted words are merged into the dictionary; Abort
(including Ctrl-C) discards pending accepted words and source edits. Case is
preserved, existing flags and entry order are retained, and duplicate entries are
not added. Confirmed affix choices are saved as exact `stem/FLAGS` entries, not as
separate copies of their expanded forms. A compatible prefix/suffix pair keeps
both flags: for example, `Zzyzxqroot/UD` can generate `Zzyzxqroot`, `unZzyzxqroot`,
`Zzyzxqrooted`, and `unZzyzxqrooted` when `/U` adds `un` and `/D` adds `ed`, with
both rules allowing cross-product (`Y`). Plain-word acceptance remains available.
Stem editors accept only single plain words, not phrases or typed Hunspell
flag/escape syntax; affix flags come from the dictionary's supported catalog.

Before writing, flagged entries are re-expanded and checked against their
confirmed forms. Source edits are then applied before vocabulary is saved. A source-update failure
stops the save and reload. Saving re-reads and merges the dictionary under its
lock, reads it back for verification, then rebuilds the spelling engines from
configuration and checks that each engine accepts the learned words. A newly
created automatic target is registered with all active spelling backends before
this reload, without rewriting TOML. Save,
verification, and reload errors are reported, but earlier
writes are not rolled back: this is **not a cross-file transaction**. Pending
acceptance uses a shared session overlay, not per-word engine mutation.

Commit the dictionary so teammates and CI use the same vocabulary. Ignore the
local sidecar lock with `*.dic.cargo-spellcheck.lock` in `.gitignore`; the lock is
intentionally left in place to coordinate concurrent sessions safely. See
[learning accepted words](configuration.md#learning-accepted-words) for validation
and persistence details.

## Examples

### Missing word variants

Sometimes some word forms belong into topic specific lingo and as such should be added to
the topic specific dictionary. Make use of suffix patterns such as `/S` for plural `s` and `/M` for `'s`. This will keep your dictionary to a minimum. Please check the [affix file included here](./hunspell-data/en_US.aff) or your OS'  provided affix file.
[It is required to understand the slightly arcane format of `.aff` and `.dic` files.](https://www.systutorials.com/docs/linux/man/4-hunspell/#lbAE) which is also available via `man 4 hunspell`.

### Types in doc comments

```raw
lib.rs : 2
 858 |  See [MmrLeafVersion] type documentation for more details.
     |       ^^^^^^^^^^^^^^
     |   Possible spelling mistake found.
```

can be resolved by using

```md
[`MmrLeafVersion`]
```

with additional ticks.

This is a general pattern for _types_ that make an appearance in the doc comments.

### Patterns

In some cases it's a pattern one wants to whitelist, such `10x` or `117x` which can be done via
the configuration adding a allowlist regex `^[0-9]+x$`.


### TODO, XXX, and FIXME

Should not be present in doc comments, but only make it into developer comments, i.e. `// FIXME foo` or `/* FIXME foo */`

### markdown: autolink


```raw
error: spellcheck(Hunspell)
   --> test.md:96
    |
 96 | The test coverage in `lcov` can the be published to <codecov.io>.
    |                                                      ^^^^^^^
    | - codec
    |
    |   Possible spelling mistake found.
```

will spellcheck all components of the url, since it is not a _valid_ autolink. Add the protocol type.

```md
<https://codecov.io>
```

and the content will be omitted from spellchecking.
