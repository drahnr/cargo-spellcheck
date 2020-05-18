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
```


## Features

* [x] Decent error printing
* [x] `cargo-spellcheck check`
* [x] spell checking using `hunspell`
* [ ] Merge multiline doc comments
* [ ] Grammar check using `languagetool` http API
* [ ] False positive reduction
* [ ] `cargo-spellcheck fix`
* [ ] `cargo-spellcheck fix --interactive`
* [ ] Re-wrap doc comments
* [ ] Word split validation
