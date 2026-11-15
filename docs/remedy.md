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

During an interactive fix session, select the `{word} < learn` row to keep the
source unchanged and choose a suffix (or the plain word). Suffix selection offers
shortened `stem/FLAG` candidates, then an editable stem and a full expansion
preview. Only confirmation records the entry. The `a` shortcut also offers this
flow: Enter or `p` continues to suffix selection.

This works with Hunspell, Spellbook, and Zet, including Rust-only builds.
Confirmed expanded forms are immediately recognized across all enabled spelling
backends, including queued diagnostics and later files; grammar diagnostics remain
unlearnable. Custom replacements edit the source and offer optional learning for
unknown words; skipping learning does not discard the source edit.

Acceptance is session-only unless you explicitly set `learning_dictionary` under
an enabled `[Hunspell]`, `[Spellbook]`, or `[ZSpell]` (Zet) section. The setting remains
per-backend; there is no top-level TOML setting. cargo-spellcheck never guesses a
target from `extra_dictionaries` and never rewrites your TOML file.

For persistent project vocabulary, create `.config/lingo.dic` first, containing
`0` followed by a newline for an empty dictionary. For `--checkers=hunspell`,
configure:

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
configuration file and must already exist. If multiple spelling backends are
enabled, configure the target under any one of them, include that same dictionary
in **every enabled spelling backend's** `extra_dictionaries`, and use the same
`lang` for each. Multiple `learning_dictionary` settings must resolve to the same
file; conflicting targets, missing shared extra dictionaries, or differing
languages are errors.

On normal completion or `q`, accepted words are merged into the dictionary; Abort
(including Ctrl-C) discards pending accepted words and source edits. Case is
preserved, existing flags and entry order are retained, and duplicate entries are
not added. Confirmed suffix choices are saved as exact `stem/FLAG` entries, not as
separate copies of their expanded forms. Plain-word acceptance remains available.
Stem editors accept only single plain words, not phrases or typed Hunspell
flag/escape syntax; suffix flags come from the dictionary's supported catalog.

Before writing, flagged entries are re-expanded and checked against their
confirmed forms. Source edits are then applied before vocabulary is saved. A source-update failure
stops the save and reload. Saving re-reads and merges the dictionary under its
lock, reads it back for verification, then rebuilds the spelling engines from
configuration and checks that each engine accepts the learned words. Save,
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
