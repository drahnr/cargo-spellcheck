# Checkers

Available checker support

## Tokenization and grammar features

The `hunspell`, `spellbook`, and `zet` features all use a shared `tokenizer`
feature. It uses the existing `nlprule` tokenizer to split prose into words;
the spelling engines themselves do not perform that segmentation.

The `nlprules` feature adds the grammar checker on top of `tokenizer`. Hunspell
retains its existing feature behavior: enabling `hunspell` also enables
`nlprules` and the `nlprule-build` build dependency, even with default features
disabled.

The Rust backends, `spellbook` and `zet`, can instead build with only `tokenizer`,
without enabling `nlprules` or `nlprule-build`. Those spelling-only builds unpack
only the bundled tokenizer data, not the grammar rules. For example:

```sh
cargo build --no-default-features --features spellbook
cargo build --no-default-features --features zet
```

These builds still depend on the `nlprule` runtime crate and its tokenizer data;
this separation does not replace the tokenizer or change its licensing.
Default builds continue to include all checkers, including grammar checking.

## Hunspell

Requires a C++ compiler to compile the hunspell CXX source files which are part
of `hunspell-sys`

### Fedora 30+

```sh
dnf install -y clang
```

### Ubuntu 19.10+

```sh
apt install -y clang
```

### Mac OS X

```sh
brew install llvm
```

The environment variable `LLVM_CONFIG_PATH` needs to point to `llvm-config`, to
do so:

```sh
export LLVM_CONFIG_PATH=/usr/local/opt/llvm/bin/llvm-config
```

## NlpRules

When compiled with the default featureset which includes `nlprules`, the
resulting binary can only be distributed under the [`LGPLv2.1`](./LICENSE-LGPL)
since the `rules` and `tokenizer` definitions are extracted from `LanguageTool`
(which is itself licensed under [`LGPLv2.1`](./LICENSE-LGPL)) as described by
the library that is used for pulling and integrating - details are to be found
under [crate `nlprule`'s
README.md](https://github.com/bminixhofer/nlprule#license).

