# Configuration

## Source

There are various ways to specify the configuration. The prioritization is as
follows:

_Explicit_ specification:

1. Command line flags `--cfg=...`.
1. `Cargo.toml` package metadata

    ```toml
    [package.metadata.spellcheck]
    config = "somewhere/cfg.toml"
    ```

1. `Cargo.toml` workspace metadata

    ```toml
    [workspace.metadata.spellcheck]
    config = "somewhere/else/cfg.toml"
    ```

which will fail if specified and not existent on the filesystem.

If neither of those ways of specification is present, continue with the
_implicit_.

1. `Cargo.toml` metadata in the current working directory `CWD`.
1. Check the first arguments location if present, else the current working directory for `.config/spellcheck.toml`.
1. Fallback to per user configuration files:
    * Linux:   `/home/alice/.config/cargo_spellcheck/config.toml`
    * Windows: `C:\Users\Alice\AppData\Roaming\cargo_spellcheck\config.toml`
    * macOS:   `/Users/Alice/Library/Preferences/cargo_spellcheck/config.toml`
1. Use the default, builtin configuration (see `config` sub-command).

Since this is rather complex, add `-vv` to your invocation to see the `info`
level logs printed, which will contain the config path.
### Format

```toml
# Project settings where a Cargo.toml exists and is passed
# ${CARGO_MANIFEST_DIR}/.config/spellcheck.toml

# Also take into account developer comments
dev_comments = false

# Skip the README.md file as defined in the cargo manifest
skip_readme = false

[Hunspell]
# lang and name of `.dic` file
lang = "en_US"
# OS specific additives
# Linux: [ /usr/share/myspell ]
# Windows: []
# macOS [ /home/alice/Libraries/hunspell, /Libraries/hunspell ]

# Additional search paths, which take presedence over the default
# os specific search dirs, searched in order, defaults last
# search_dirs = []

# Adds additional dictionaries, can be specified as
# absolute paths or relative in the search dirs (in this order).
# Relative paths are resolved relative to the configuration file
# which is used.
# Refer to `man 5 hunspell`
# or https://www.systutorials.com/docs/linux/man/4-hunspell/#lbAE
# on how to define a custom dictionary file.
extra_dictionaries = []

# If set to `true`, the OS specific default search paths
# are skipped and only explicitly specified ones are used.
skip_os_lookups = false

# Use the builtin dictionaries if none were found in
# in the configured lookup paths.
# Usually combined with `skip_os_lookups=true`
# to enforce the `builtin` usage for consistent
# results across distributions and CI runs.
# Setting this will still use the dictionaries
# specified in `extra_dictionaries = [..]`
# for topic specific lingo.
use_builtin = true


[Hunspell.quirks]
# Transforms words that are provided by the tokenizer
# into word fragments based on the capture groups which are to
# be checked.
# If no capture groups are present, the matched word is whitelisted.
# All expressions can also match across tokenizer boundaries within one
# non-whitespace sequence, e.g. compounds containing "->", "-+>", or "::".
# Captures select fragments to check, including fragments spanning tokens;
# without captures, only the matching portion is excluded, not neighboring words.
# Rules are ordered: earlier matches on an original token take precedence over
# later cross-token matches. See "Transformation rule precedence" below.
transform_regex = ["^'([^\\s])'$", "^[0-9]+x$"]
# Accepts `alphabeta` variants if the checker provides a replacement suggestion
# of `alpha-beta`.
allow_concatenation = true
# And the counterpart, which accepts words with dashes, when the suggestion has
# recommendations without the dashes. This is less common.
allow_dashed = false
# Check the expressions in the footnote references. By default this is turned on
# to remain backwards compatible but disabling it could be particularly useful
# when one uses abbreviations instead of numbers as footnote references.  For
# instance by default the fragment `hello[^xyz]` would be spellchecked as
# `helloxyz` which is obviously a misspelled word, but by turning this check
# off, it will skip validating the reference altogether and will only check the
# word `hello`.
check_footnote_references = false

[NlpRules]
# Allows the user to override the default included
# exports of LanguageTool, with other custom
# languages

# override_rules = "/path/to/rules_binencoded.bin"
# override_tokenizer = "/path/to/tokenizer_binencoded.bin"

[Reflow]
# Reflows doc comments to adhere to adhere to a given maximum line width limit.
max_line_length = 80
```

### Transformation rule precedence

`transform_regex` uses array order for Hunspell, Spellbook, and ZSpell:

- For each tokenizer token, the first matching expression determines the
  transformation. Capture groups select fragments to check; an expression without
  capture groups allow-lists the whole matching token. Captured fragments are
  recursively transformed using the same ordered rules. Identity captures (those
  selecting the entire fragment unchanged) advance to later rules to terminate.
- All expressions can also match across two or more tokenizer tokens within one
  non-whitespace sequence. Without capture groups, only the eligible matching
  portion is excluded. With capture groups, each nonempty participating group is
  a separate check candidate, including a capture spanning multiple tokens. Named
  captures use normal regex syntax. Uncaptured portions of the full match
  (group 0) are skipped, while neighbors outside the match remain available for
  checking. Source offsets are preserved. Compound processing accepts only
  non-overlapping full matches in source order; overlapping matches reported by
  advanced constructs such as `\K` in lookbehind are skipped.
- For compound matches, captures are clipped to group 0; lookaround cannot select
  text outside the match. Existing per-token lookaround behavior is unchanged.
  Empty captures and unmatched optional groups are ignored. For compound matches,
  if no nonempty captures remain, the eligible matched portion is skipped.
  Existing per-token rules with only empty or absent captures still leave the
  token unchanged. Overlapping or nested captures are independent candidates;
  identical capture ranges are deduplicated before recursive transformation.
- An earlier expression matching an **original token** protects that entire token
  from a later cross-token match, even if the match overlaps only part of the
  token. Other, unprotected tokens overlapped by the same match can still be
  transformed. Ranges selected by an earlier cross-token capture are protected
  from later cross-token exclusions. Conversely, an earlier exclusion is never
  restored by a later capture rule.

This protection applies to compound preprocessing, not to subsequent per-fragment
transformations. Captures and remaining token fragments are then transformed as
standalone strings: anchors and lookarounds see fragment boundaries rather than
the original surrounding text. A rule that did not match the original token may
therefore match its remaining fragment. Recursive rules can also allow-list a
spanning capture (for example, a rule matching its entire captured text).

For example, with tokens `reciever-`, `>`, and `message`, this order preserves the
`reciever` spelling diagnostic because the first rule selects it for checking:

```toml
[Hunspell.quirks]
transform_regex = ["(reciever)-", "reciever->"]
```

Reversing these two rules allow-lists `reciever->`, so `reciever` is not checked.
In either order, the neighboring `message` token is still checked.

To check both sides of compounds such as `foo::bar` and `foo->bar`, use multiple
captures with generic letter patterns rather than matching specific words:

```toml
[Hunspell.quirks]
transform_regex = ['(?P<left>[A-Za-z]+)(?:::|->)(?P<right>[A-Za-z]+)']
```

Each named group is checked separately; the separator is skipped. To select a
spanning capture first, place `'(?P<compound>[A-Za-z]+(?:::|->)[A-Za-z]+)'` before
that rule. The captured compound is recursively transformed, advancing past the
identity capture to the subsequent rule that splits it into two candidates.

To increase verbosity add `-v` (multiple) to increase verbosity.
