# Conditional documentation

`cargo spellcheck` checks documentation in `#[cfg_attr(..., doc = "...")]`,
including nested `cfg_attr` attributes. It checks alternative documentation
without evaluating predicates against the current machine or enabled Cargo
features. You do not need to build for every target to check its prose.

## Alternative Markdown fragments

Documentation comments and `doc` string attributes on the same item form an
ordered documentation run. For each symbolic configuration, the selected
fragments are joined before Markdown is processed. Mutually exclusive code
fences therefore remain alternatives, rather than opening two nested fences:

````rust
/// # Example
///
#[cfg_attr(unix, doc = "```no_run")]
#[cfg_attr(not(unix), doc = "```ignore")]
/// let example_code_is_not_prose = 42;
/// ```
///
/// This explanation is checked in both alternatives.
struct Example;
````

Both versions retain the heading and explanation while excluding the fenced
code from spellchecking. A diagnostic for shared source prose is reported only
once per checker when its source span, replacements, and description match.
Identical prose at different source locations is still checked separately.

## Nested conditions and correlation

Supported predicates are flags such as `unix`, string-valued keys such as
`feature = "extra"`, `all(...)`, `any(...)`, `not(...)`, and Boolean literals
`true` and `false`. Empty `all()` is true; empty `any()` is false. Repeated
predicates remain correlated across attributes, and nested conditions are
conjoined with their parents:

```rust
/// Introduction.
#[cfg_attr(feature = "extra",
    doc = "Extra explanation.",
    cfg_attr(feature = "details", doc = "Detailed explanation."),
    doc = "Extra conclusion.",
)]
struct Nested;
```

This produces three distinct documentation versions: the introduction alone,
the introduction with the extra explanation and conclusion, and that same
version with the detailed explanation inserted in order. Assignments selecting
the same source fragments are checked only once. Raw identifier spellings such
as `r#doc` and `r#cfg_attr`, raw strings, and multiline strings are supported.
String source text is preserved for accurate correction spans; Rust escapes
are not decoded and documentation macros such as `include_str!` and `concat!`
are not expanded.

## Boundaries and safe reflow

Unrelated attributes may separate documentation fragments without splitting
the Markdown run:

```rust
/// Before the attribute.
#[allow(dead_code)]
/// After the attribute.
struct Bridged;
```

Runs do not cross ordinary item tokens, scope boundaries, or a change between
inner (`#![...]` / `//!`) and outer (`#[...]` / `///`) attributes. Attribute-like
text inside unrelated attribute payloads is not treated as documentation.

Spelling suggestions keep their original source spans, including across gaps.
However, reflow skips conditional runs, structurally gapped runs, mixed comment
formats, and attribute prefixes that cannot safely be repeated. Rewriting
these as one contiguous block could erase Rust attributes or change which
configuration receives the documentation. Independent safe documentation
blocks in the same file can still be reflowed normally.

## Limits

Configuration atoms are symbolic and independent: the checker does not infer
platform exclusivity or relationships between different key values. Predicates
repeated in one run share an atom; Boolean constants do not consume atoms.
To bound exponential enumeration, a run with more than ten distinct atoms is
skipped with a warning. Unsupported predicate syntax or an invalid supported
attribute also causes its whole documentation run to be skipped with a warning,
rather than checking an incomplete Markdown projection. Other runs in the file
remain checkable. Ordinary `#[cfg(...)]` does not filter documentation against
the host configuration.
