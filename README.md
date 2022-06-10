# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)
[![CI](https://ci.spearow.io/api/v1/teams/main/pipelines/cargo-spellcheck/jobs/master-validate/badge)](https://ci.spearow.io/teams/main/pipelines/cargo-spellcheck/jobs/master-validate)
![commits-since](https://img.shields.io/github/commits-since/drahnr/cargo-spellcheck/latest.svg)
[![rust 1.57.0+ badge](https://img.shields.io/badge/rust-1.57.0+-93450a.svg)](https://blog.rust-lang.org/2021/12/02/Rust-1.57.0.html)

Check your spelling with `hunspell` and/or `nlprule`.

## Use Cases

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your
documentation comments in order to avoid nasty typos all over your source tree.
Meant as a helper simplifying review as well as improving CI checks after a
learning phase for custom/topic specific lingo.

`cargo-spellcheck` is also a valuable tool to run from git commit hooks or CI/CD
systems.

### Check For Spelling and/or Grammar Mistakes

```zsh
cargo spellcheck check
```

<pre><code><span style="color:#CC0000"><b>error</b></span><span style="color:#D3D7CF"><b>: spellcheck</b></span>
<span style="color:#3465A4">   --&gt;</span> src/main.rs:44
<span style="color:#3465A4"><b>    |</b></span>
<span style="color:#3465A4"><b> 44 |</b></span> Fun facets shalld cause some erroris.
<span style="color:#3465A4"><b>    |</b></span><span style="color:#C4A000"><b>            ^^^^^^</b></span>
<span style="color:#3465A4"><b>    |</b></span><span style="color:#CC0000"><b> - </b></span><span style="color:#4E9A06"><b>shall</b></span> or <span style="color:#4E9A06">shall d</span>
<span style="color:#3465A4"><b>    |</b></span></code></pre>

### Apply Suggestions Interactively

```zsh
cargo spellcheck fix
```

<pre><code><span style="color:#CC0000"><b>error</b></span><span style="color:#D3D7CF"><b>: spellcheck(Hunspell)</b></span>
<span style="color:#3465A4">    --&gt;</span> /media/supersonic1t/projects/cargo-spellcheck/src/literalset.rs:291
<span style="color:#3465A4"><b>     |</b></span>
<span style="color:#3465A4"><b> 291 |</b></span>  Returns literl within the Err variant if not adjacent
<span style="color:#3465A4"><b>     |</b></span><span style="color:#C4A000"><b>          ^^^^^^</b></span>

<span style="color:#729FCF"><b>(13/14) Apply this suggestion [y,n,q,a,d,j,e,?]?</b></span>

   <span style="background-color:#2E3436;color:#729FCF;">lite</span>
   <span style="background-color:#2E3436;color:#729FCF;">litter</span>
   <span style="background-color:#2E3436;color:#729FCF;">litterer</span>
   <span style="background-color:#2E3436;color:#729FCF;">liter l</span>
   <span style="background-color:#2E3436;color:#729FCF;">liters</span>
   <span style="background-color:#2E3436;color:#729FCF;">literal</span>
   <span style="background-color:#2E3436;color:#729FCF;">liter</span>
 <span style="color:#8AE234"><b>»</b></span> <span style="background-color:#2E3436;color:#FCE94F">a custom replacement literal</span></code></pre>

## Installation

`cargo install --locked cargo-spellcheck`

The `--locked` flag is the preferred way of installing to get the tested set of
dependencies.

## Completions

`cargo spellcheck completions` for autodetection of your current shell via
`$SHELL`,

 or

`cargo spellcheck completions --shell zsh`

to explicitly specify your shell type.

Commonly it's use like this from your shell's `.rc*` file:

`source <(cargo spellcheck completion)`

Note: There is a [relevant clap issue
(#3508)](https://github.com/clap-rs/clap/issues/3508) that makes this fail in
some cases.

## 🎈 Contribute!

Contributions are very welcome!

Generally the preferred way of doing so, is to comment in an issue that you
would like to tackle the implementation/fix.

This is usually followed by an initial PR where the implementation is then
discussed and iteratively refined. No need to get it all correct
the first time!

## Documentation

- [Features and Roadmap](docs/features.md)
- [Remedies for common issues](docs/remedy.md)
- [Configuration](docs/configuration.md)
- [Available Checkers](docs/checkers.md)
- [Automation of `cargo-spellcheck`](docs/automation.md)
