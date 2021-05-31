# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)
[![CI](https://ci.spearow.io/api/v1/teams/main/pipelines/cargo-spellcheck/jobs/master-validate/badge)](https://ci.spearow.io/teams/main/pipelines/cargo-spellcheck/jobs/master-validate)
![commits-since](https://img.shields.io/github/commits-since/drahnr/cargo-spellcheck/latest.svg)
Check your spelling with `hunspell` and/or `nlprule`.

## Use Cases

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your
documentation comments in order to avoid nasty typos all over your source tree.
Meant as a helper simplifying review as well as improving CI checks after a
learning phase for custom/topic specific lingo.

### Check For Spelling and/or Grammar Mistakes

```zsh
cargo spellcheck check
```

<pre><font color="#CC0000"><b>error</b></font><font color="#D3D7CF"><b>: spellcheck</b></font>
<font color="#3465A4">   --&gt;</font> src/main.rs:44
<font color="#3465A4"><b>    |</b></font>
<font color="#3465A4"><b> 44 |</b></font> Fun facets shalld cause some erroris.
<font color="#3465A4"><b>    |</b></font><font color="#C4A000"><b>            ^^^^^^</b></font>
<font color="#3465A4"><b>    |</b></font><font color="#CC0000"><b> - </b></font><font color="#4E9A06"><b>shall</b></font> or <font color="#4E9A06">shall d</font>
<font color="#3465A4"><b>    |</b></font>
</pre>

### Apply Suggestions Interactively

```zsh
cargo spellcheck fix
```

<pre><font color="#CC0000"><b>error</b></font><font color="#D3D7CF"><b>: spellcheck(Hunspell)</b></font>
<font color="#3465A4">    --&gt;</font> /media/supersonic1t/projects/cargo-spellcheck/src/literalset.rs:291
<font color="#3465A4"><b>     |</b></font>
<font color="#3465A4"><b> 291 |</b></font>  Returns literl within the Err variant if not adjacent
<font color="#3465A4"><b>     |</b></font><font color="#C4A000"><b>          ^^^^^^</b></font>

<font color="#729FCF"><b>(13/14) Apply this suggestion [y,n,q,a,d,j,e,?]?</b></font>

   <span style="background-color:#2E3436"><font color="#729FCF">lite</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">litter</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">litterer</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">liter l</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">liters</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">literal</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">liter</font></span>
 <font color="#8AE234"><b>»</b></font> <span style="background-color:#2E3436"><font color="#FCE94F">a custom replacement literal</font></span>
</pre>

### Continuous Integration / CI

`cargo spellcheck` can be configured with `-m <code>` to return a non-zero
return code if mistakes are found instead of `0`.

### git pre-commit hook

```sh
#!/usr/bin/sh

# Redirect output to stderr.
exec 1>&2

exec cargo spellcheck -m 99 $(git diff-index --cached --name-only --diff-filter=AM HEAD)
```

## Implemented Features + Roadmap

* [x] Parse doc comments from arbitrary files
* [x] Decent error printing
* [x] `cargo-spellcheck check`
* [x] Spell checking using `hunspell`
* [x] Merge multiline doc comments
* [x] Handle multiline and fragmented mistakes (i.e. for grammar) [#25](https://github.com/drahnr/cargo-spellcheck/issues/25)
* [x] Grammar check using [`nlprule`](https://github.com/bminixhofer/nlprule)
* [x] Follow module declarations rather than blindly recurse
* [x] Be `commonmark`/`markdown` aware
  * [ ] Handle doc-tests with ` ```rust` as virtual files [#43](https://github.com/drahnr/cargo-spellcheck/issues/43)
  * [ ] Verify all types of links [#44](https://github.com/drahnr/cargo-spellcheck/issues/44)
* [x] Check `README.md` files [#37](https://github.com/drahnr/cargo-spellcheck/issues/37)
* [x] Improve interactive user interface with `crossterm`
* [x] Ellipsize overly long statements with `...` [#42](https://github.com/drahnr/cargo-spellcheck/issues/42)
* [ ] Learn topic lingo and filter false-positive-suggestions [#41](https://github.com/drahnr/cargo-spellcheck/issues/41)
* [x] Handle cargo workspaces [#38](https://github.com/drahnr/cargo-spellcheck/issues/38)
* [x] Re-flow doc comments [#39](https://github.com/drahnr/cargo-spellcheck/issues/39)
* [x] Collect dev comments as well [#115](https://github.com/drahnr/cargo-spellcheck/issues/115)

`hunspell` (dictionary based lookups) and `nlprules` (static grammar rules,
derived from `languagetool`) are currently the two supported checkers.

## Configuration

```toml
# Project settings where a Cargo.toml exists and is passed
# ${CARGO_MANIFEST_DIR}/.config/spellcheck.toml

# Also take into account developer comments
dev_comments = false

# Skip the README.md file as defined in the cargo manifest
skip_readme = false

# Fallback to per use configuration files:
# Linux:   /home/alice/.config/cargo_spellcheck/config.toml
# Windows: C:\Users\Alice\AppData\Roaming\cargo_spellcheck\config.toml
# macOS:   /Users/Alice/Library/Preferences/cargo_spellcheck/config.toml



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

## Installation

`cargo install --locked cargo-spellcheck`

The `--locked` flag is the preferred way of installing to get the tested set of
dependencies.

### Checkers

Available checker support

#### Hunspell

Requires a C++ compiler to compile the hunspell CXX source files which are part
of `hunspell-sys`

##### Fedora 30+

```sh
dnf install -y clang
```

##### Ubuntu 19.10+

```sh
apt install -y clang
```

##### Mac OS X

```sh
brew install llvm
```

The environment variable `LLVM_CONFIG_PATH` needs to point to `llvm-config`, to
do so:

```sh
export LLVM_CONFIG_PATH=/usr/local/opt/llvm/bin/llvm-config
```

#### NlpRules

When compiled with the default featureset which includes `nlprules`, the
resulting binary can only be distributed under the [`LGPLv2.1`](./LICENSE-LGPL)
since the `rules` and `tokenizer` definitions are extracted from `LanguageTool`
(which is itself licensed under [`LGPLv2.1`](./LICENSE-LGPL)) as described by
the library that is used for pulling and integrating - details are to be found
under [crate `nlprule`'s
README.md](https://github.com/bminixhofer/nlprule#license).

## 🎈 Contribute!

Contributions are very welcome!

Generally the preferred way of doing so, is to comment in an issue that you
would like to tackle the implementation/fix.

This is usually followed by an initial PR where the implementation is then
discussed and iteratively refined. No need to get it all correct
the first time!
