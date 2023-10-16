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

To increase verbosity add `-v` (multiple) to increase verbosity.
