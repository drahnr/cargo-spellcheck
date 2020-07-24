# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)
[![CI](https://ci.spearow.io/api/v1/teams/main/pipelines/cargo-spellcheck/jobs/master-validate/badge)](https://ci.spearow.io/teams/main/pipelines/cargo-spellcheck/jobs/master-validate)
![commits-since](https://img.shields.io/github/commits-since/drahnr/cargo-spellcheck/latest.svg)

Check your spelling with `hunspell` and/or `languagetool`.

## Use Cases

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your documentation comments
in order to avoid narsty types all over the place.

Meant as a helper simplifying review as well as possibly improving CI
after a learning phase for custom/topic specifc lingo.

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
cargo spellcheck fix --interactive
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

`cargo spellcheck` can be configured with `-m <code>` to return a non-zero return code if
mistakes are found instead of `0`.

## Implemented Features + Roadmap

* [x] Parse doc comments from arbitrary files
* [x] Decent error printing
* [x] `cargo-spellcheck check`
* [x] Spell checking using `hunspell`
* [x] Merge multiline doc comments
* [x] Handle multiline and fragmented mistakes (i.e. for grammar) [#25](https://github.com/drahnr/cargo-spellcheck/issues/25)
* [x] Grammar check using `languagetool` http API
* [x] Follow module declarations rather than blindly recurse
* [x] Be `markdown` aware
  * [ ] Handle doctests with ` ```rust` as virtual files [#43](https://github.com/drahnr/cargo-spellcheck/issues/43)
  * [ ] Verify all types of links [#44](https://github.com/drahnr/cargo-spellcheck/issues/44)
* [ ] Check `README.md` files [#37](https://github.com/drahnr/cargo-spellcheck/issues/37)
* [ ] Check mdbook `book.toml` file trees [#62](https://github.com/drahnr/cargo-spellcheck/issues/62)
* [x] `cargo-spellcheck fix --interactive`
* [x] Improve interactive user interface with `crossterm`
* [x] Ellipsize overly long statements with `...` [#42](https://github.com/drahnr/cargo-spellcheck/issues/42)
* [ ] Learn topic lingo and filter false-positive-suggestions when `fix --interactive` is passed [#41](https://github.com/drahnr/cargo-spellcheck/issues/41)
* [x] Handle cargo workspaces [#38](https://github.com/drahnr/cargo-spellcheck/issues/38)
* [ ] Re-wrap doc comments [#39](https://github.com/drahnr/cargo-spellcheck/issues/39)
* [ ] Word split validation [#40](https://github.com/drahnr/cargo-spellcheck/issues/40)
* [ ] `cargo-spellcheck fix` [#45](https://github.com/drahnr/cargo-spellcheck/issues/45)

`hunspell` and `languagetool` are currently the two supported featuresets.

## Configuration

```toml
# Linux:   /home/alice/.config/cargo_spellcheck/config.toml
# Windows: C:\Users\Alice\AppData\Roaming\cargo_spellcheck\config.toml
# macOS:   /Users/Alice/Library/Preferences/cargo_spellcheck/config.toml
[LanguageTool]
url = "127.0.0.1:8010"

[Hunspell]
# lang and name of `.dic` file
lang = "en_US"
# OS specific additives
# Linux: [ /usr/share/myspell ]
# Windows: []
# macOS [ /home/alice/Libraries/hunspell, /Libraries/hunspell ]
search_dirs = []
extra_dictonaries = []
```

To increase verbosity use `CARGO_SPELLCHECK=cargo_spellcheck=trace` to see internal details or
add `-v` (multiple) to increase verbosity.

## Installation

`cargo install cargo-spellcheck`

### Checkers

Available checker support

#### Hunspell

Requires a C++ compiler to compile the hunspell CXX source files which are part of `hunspell-sys`

##### Fedora 30+
```sh
dnf install -y clang
```

##### Ubuntu 19.10+
```sh
apt install -y clang
```

##### Mac OS X
```
brew install llvm
```

The environment variable `LLVM_CONFIG_PATH` needs to point to `llvm-config`, to do so:

```sh
export LLVM_CONFIG_PATH=/usr/local/opt/llvm/bin/llvm-config
```

#### LanguageTool

Run a instance of the [LanguageTool server i.e. as container](https://hub.docker.com/r/erikvl87/languagetool).
