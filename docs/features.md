# Implemented Features + Roadmap

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

