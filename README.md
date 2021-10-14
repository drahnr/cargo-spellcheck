# cargo-spellcheck

[![crates.io](https://img.shields.io/crates/v/cargo_spellcheck.svg)](https://crates.io/crates/cargo-spellcheck)
[![CI](https://ci.spearow.io/api/v1/teams/main/pipelines/cargo-spellcheck/jobs/master-validate/badge)](https://ci.spearow.io/teams/main/pipelines/cargo-spellcheck/jobs/master-validate)
![commits-since](https://img.shields.io/github/commits-since/drahnr/cargo-spellcheck/latest.svg)
[![rust 1.51.0+ badge](https://img.shields.io/badge/rust-1.51.0+-93450a.svg)](https://blog.rust-lang.org/2021/03/25/Rust-1.51.0.html)

Check your spelling with `hunspell` and/or `nlprule`.

## Use Cases

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your
documentation comments in order to avoid nasty typos all over your source tree.
Meant as a helper simplifying review as well as improving CI checks after a
learning phase for custom/topic specific lingo.

`cargo-cpellcheck` is also a valuable tool to run from git commit hooks or CI/CD
systems.

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

## Installation

`cargo install --locked cargo-spellcheck`

The `--locked` flag is the preferred way of installing to get the tested set of
dependencies.

## 🎈 Contribute!

Contributions are very welcome!

Generally the preferred way of doing so, is to comment in an issue that you
would like to tackle the implementation/fix.

This is usually followed by an initial PR where the implementation is then
discussed and iteratively refined. No need to get it all correct
the first time!

## Documentation

- [Features and Roadmap](docs/features.md)
- [Remedies for common issues](docs/remedies.md)
- [Configuration](docs/configuration.md)
- [Availabble Checkers](docs/checkers.md)
- [Automation of `cargo-spellcheck`](docs/automation.md)
