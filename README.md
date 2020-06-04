# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)

Check your spelling with `hunspell` and/or `languagetool`.

## Usecase

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your documentation comments
in order to avoid narsty types all over the place.

Meant as a helper simplifying review as well as possibly improving CI
after a learning phase for custom/topic specifc lingo.
`cargo spellcheck` has a return code `1` if any unknown words are found, and `0` on success.

Error display follows `cargo` error printing style:

<pre><font color="#CC0000"><b>error</b></font><font color="#D3D7CF"><b>: spellcheck</b></font>
<font color="#3465A4">   --&gt;</font> src/main.rs:44
<font color="#3465A4"><b>    |</b></font>
<font color="#3465A4"><b> 44 |</b></font> Fun facets shalld cause some erroris.
<font color="#3465A4"><b>    |</b></font><font color="#C4A000"><b>            ^^^^^^</b></font>
<font color="#3465A4"><b>    |</b></font><font color="#CC0000"><b> - </b></font><font color="#4E9A06"><b>shall</b></font> or <font color="#4E9A06">shall d</font>
<font color="#3465A4"><b>    |</b></font>
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
  * [ ] Handle doctests with ` ```rust` as virtual files [skeptic-like](https://github.com/budziq/rust-skeptic/blob/master/src/skeptic/lib.rs#L240-L259) which would be straight forward
  * [ ] Verify all types of links: direct urls and href
* [ ] Check `README.md` files
* [ ] `cargo-spellcheck fix --interactive`
* [ ] `cargo-spellcheck fix`
* [ ] Ellipsize overly long statements with `...`
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
dnf install -y hunspell-devel
```

and building should succeed just fine.

### LanguageTool

Run a instance of the [LanguageTool server i.e. as container](https://hub.docker.com/r/erikvl87/languagetool) .
