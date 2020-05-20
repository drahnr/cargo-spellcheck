# cargo-spellcheck

**WIP** Check your spelling. **WIP**

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
* [ ] Be `markdown` aware
* [ ] Exclude \`\`\` wrapped sections
* [ ] Handle module documentation comments
* [ ] Add `README.md` files
* [ ] Follow module declarations rather than blindly recurse
* [ ] `cargo-spellcheck fix`
* [ ] `cargo-spellcheck fix --interactive`
* [ ] Re-wrap doc comments
* [ ] Word split validation

`hunspell` and `languagetool` are currently the two supported featuresets.


## Installation

`cargo install cargo-spellcheck`

### Hunspell

Requires the native library

```sh
dnf install -y hunspell-devel
```

and building should succeed just fine.

### LanguageTool

Run a instance of the [LanguageTool server i.e. as container](https://hub.docker.com/r/erikvl87/languagetool) .