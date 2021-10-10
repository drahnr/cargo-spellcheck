# Automation of `cargo-spellcheck`

## CI/CD

`cargo-spellcheck` can be configured with `-m <code>` to return a non-zero
return code if mistakes are found instead of `0`.

## Git hooks

If you want to manually configure `cargo-spellcheck` to run on git commits:

```sh
#!/usr/bin/sh

# Redirect output to stderr.
exec 1>&2

exec cargo spellcheck -m 99 $(git diff-index --cached --name-only --diff-filter=AM HEAD)
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
