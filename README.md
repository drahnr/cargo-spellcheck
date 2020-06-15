# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)
[![CI](https://ci.spearow.io/api/v1/teams/main/pipelines/cargo-spellcheck/jobs/master-validate/badge)](https://ci.spearow.io/teams/main/pipelines/cargo-spellcheck/jobs/master-validate)
[![](https://img.shields.io/github/commits-since/drahnr/cargo-spellcheck/latest.svg)]()

Check your spelling with `hunspell` and/or `languagetool`.

## Usecase

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your documentation comments
in order to avoid narsty types all over the place.

Meant as a helper simplifying review as well as possibly improving CI
after a learning phase for custom/topic specifc lingo.
`cargo spellcheck` has a return code `1` if any unknown words are found, and `0` on success.

## Test Cases

### Check

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

### Interactive fixing

```zsh
cargo spellcheck fix --interactive
```

Improvement requests tracked in [issue #7](https://github.com/drahnr/cargo-spellcheck/issues/7).

<pre><font color="#CC0000"><b>error</b></font><font color="#D3D7CF"><b>: spellcheck(Hunspell)</b></font>
<font color="#3465A4">  --&gt;</font> /media/supersonic1t/projects/cargo-spellcheck/demo/src/nested/justtwo.rs:2
<font color="#3465A4"><b>   |</b></font>
<font color="#3465A4"><b> 2 |</b></font>  Beto
<font color="#3465A4"><b>   |</b></font><font color="#C4A000"><b>  ^^^^</b></font>

<font color="#729FCF"><b>(2/2) Apply this suggestion [y,n,q,a,d,j,e,?]?</b></font>

   <span style="background-color:#2E3436"><font color="#729FCF">Beeton</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Bet o</font></span>
 <font color="#8AE234"><b>»</b></font> <span style="background-color:#2E3436"><font color="#8AE234"><b>Beta</b></font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Bets</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Bet</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Be-to</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Be to</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Veto</font></span>
   <span style="background-color:#2E3436"><font color="#729FCF">Beth</font></span>
</pre>

## Features

* [x] Parse doc comments from arbitrary files
* [x] Decent error printing
* [x] `cargo-spellcheck check`
* [x] Spell checking using `hunspell`
* [x] Merge multiline doc comments
* [x] Grammar check using `languagetool` http API
* [x] False positive reduction
* [x] Follow module declarations rather than blindly recurse
* [x] Be `markdown` aware
  * [ ] Handle doctests with ` ```rust` as virtual files [skeptic-like](https://github.com/budziq/rust-skeptic/blob/master/src/skeptic/lib.rs#L240-L259)
  * [ ] Verify all types of links: direct urls and href
* [ ] Check `README.md` files
* [x] `cargo-spellcheck fix --interactive`
* [x] Improve interactive user interface with `crossterm`
* [ ] Ellipsize overly long statements with `...`
* [ ] `cargo-spellcheck fix`
* [ ] Learn topic lingo and filter false-positive-suggestions when `fix --interactive` is passed
* [ ] Handle cargo workspaces
* [ ] Re-wrap doc comments
* [ ] Word split validation

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

## Installation

`cargo install cargo-spellcheck`

To increase verbosity use `CARGO_SPELLCHECK=cargo_spellcheck=trace` to see internal details or
add `-v` (multiple) to increase verbosity.

### Hunspell

Requires the native library

```sh
# Fedora 30+
dnf install -y hunspell-devel

# Ubuntu 19.10+
apt install -y libhunspell-dev

# Mac OS X
brew install hunspell
```

and building should succeed just fine.

### LanguageTool

Run a instance of the [LanguageTool server i.e. as container](https://hub.docker.com/r/erikvl87/languagetool) .
