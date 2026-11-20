# Agent instructions

## Documentation extraction tests

- Always assert the exact resulting Markdown extracted from documentation before tokenization or Markdown stripping.
- Write expected Markdown as Rust raw string literals, with literal newlines for multiline content. Do not use escaped strings such as `"First\nSecond"`.
- Do not substitute opaque indices, counts, lengths, or other numeric assertions for assertions of the expected Markdown content. Structural assertions may supplement content assertions, not replace them.
