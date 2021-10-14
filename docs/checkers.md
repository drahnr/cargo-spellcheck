# Checkers

Available checker support

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

