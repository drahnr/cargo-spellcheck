# cargo-spellcheck

**WIP** Check your spelling. **WIP**

## Usecase

Run `cargo spellcheck --fix` or `cargo spellcheck fix` to fix all your documentation comments
in order to avoid narsty types all over the place.

Meant as a helper simplifying review as well as possibly improving CI
after a learning phase for custom/topic specifc lingo.
`cargo spellcheck` has a return code `1` if any unknown words are found, and `0` on success.

## Features

* [ ] spell checking using `hunspell`
* [ ] grammar checking using `languagetool` http API
