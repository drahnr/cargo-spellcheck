# Agent instructions

## Documentation extraction tests

- Always assert the exact resulting Markdown extracted from documentation before tokenization or Markdown stripping.
- Declare each test's input and expectations together in a single upfront fixture; do not scatter expected values through assertion branches.
- Prefer visual, literal expectations. Use Rust raw string literals for multiline inputs and expected Markdown, with literal newlines rather than escaped strings such as `"First\nSecond"`.
- When exact newline bytes are under test (for example, LF versus CRLF), use explicit escapes or conversion so IDE newline normalization cannot silently change the fixture.
- Do not substitute opaque indices, counts, lengths, or other numeric assertions for assertions of the expected Markdown content. Structural assertions may supplement content assertions, not replace them.
