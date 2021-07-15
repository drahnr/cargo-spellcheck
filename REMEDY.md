# Fixing spelling mistakes

While cargo-spellcheck is good at _pointing out_ existing spellchecks,
it's sometimes not obvious how to resolve them or what the correct way is
to resolve them.

The following covers an abstracted set of commonly encountered `cargo spellcheck`
complaints and how to resolve them:

## Configuration

Make sure your runs are idempotent if you run on two different systems,
which is easiest achieved by using the builtin affix and dictionaries
besides the topic specifc lingo dictionary that should come with your project.

```toml
# .config/spellcheck.toml

[Hunspell]
# snip
skip_os_lookups = true
use_builtin = true
# snip
```

---

Avoiding `nlprule` backend by passing `--checkers=hunspell` might be a good idea,
since `nlprule` tends to have a few false positives.

## Examples

### Missing word variants

Sometimes some word forms belong into topic specific lingo and as such should be added to
the topic specific dictionary. Make use of suffix patterns such as `/S` for plural `s` and `/M` for `'s`. This will keep your dictionary to a minimum. Please check the [affix file included here](./hunspell-data/en_US.aff) or your OS'  provided affix file.
[It is required to understand the slightly arkane format of `.aff` and `.dic` files.](https://www.systutorials.com/docs/linux/man/4-hunspell/#lbAE) which is also available via `man 4 hunspell`.

### Types in doc comments

```raw
lib.rs : 2
 858 |  See [MmrLeafVersion] type documentation for more details.
     |       ^^^^^^^^^^^^^^
     |   Possible spelling mistake found.
```

can be resolved by using

```md
[`MmrLeafVersion`]
```

with additional ticks.

This is a general pattern for _types_ that make an appearence in the doc comments.

### Patterns

In some cases it's a pattern one wants to whitelist, such `10x` or `117x` which can be done via
the configuration adding a allowlist regex `^[0-9]+x$`.


### TODO, XXX, and FIXME

Should not be present in doc comments, but only make it into developer comments, i.e. `// FIXME foo` or `/* FIXME foo */`

### markdown: autolink


```raw
error: spellcheck(Hunspell)
   --> test.md:96
    |
 96 | The test coverage in `lcov` can the be published to <codecov.io>.
    |                                                      ^^^^^^^
    | - codec
    |
    |   Possible spelling mistake found.
```

will spellcheck all components of the url, since it is not a _valid_ autolink. Add the protocol type.

```md
<https://codecov.io>
```

and the content will be omitted from spellchecking.
