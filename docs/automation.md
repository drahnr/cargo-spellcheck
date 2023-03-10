# Automation of `cargo-spellcheck`

## CI/CD

`cargo-spellcheck` can be configured with `--code <code>` to return a non-zero
return code if mistakes are found instead of `0`.

### GitHub Actions

[Create a workflow](https://docs.github.com/en/actions/quickstart) for your project and add the following example as steps.

The first step installs cargo-spellcheck on the runner.
The second step loads your source code into the runner environment.
The third step runs a command in a shell like you would normally do with cargo spellcheck.
Specify your arguments as needed.

```yaml
- name: Install cargo-spellcheck
  uses: taiki-e/install-action@v2
  with:
    tool: cargo-spellcheck
    
- uses: actions/checkout@v3

- name: Run cargo-spellcheck
  run: cargo spellcheck --code 1
```

### Other

Install `cargo-spellcheck` via [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) and then use it like you would locally.
Alternatively you can use `cargo install cargo-spellcheck` to compile it from source.

```bash
cargo binstall --no-confirm cargo-spellcheck

cargo-spellcheck --code 1
```

## Git hooks

If you want to manually configure `cargo-spellcheck` to run on git commits:

```bash
#!/usr/bin/env bash

# Redirect output to stderr.
exec 1>&2

exec cargo spellcheck --code 99 $(git diff-index --cached --name-only --diff-filter=AM HEAD)
```

Alternatively you can use [`pre-commit`](https://pre-commit.com/) to manage your git commit hooks
for you. This can be done by appending these lines to `.pre-commit-config.yaml` in your project:

```yaml
- repo: https://github.com/drahnr/cargo-spellcheck.git
  rev: master
  - id: cargo-spellcheck

```

You will need to install the hooks running `pre-commit install-hooks` and `cargo-spellcheck` will
get installed and wired up as a git commit hook for you.
