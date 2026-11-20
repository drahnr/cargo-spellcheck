# Conditional documentation

`cargo-spellcheck` reconstructs documentation alternatives selected by Rust's
`cfg` and `cfg_attr` attributes before interpreting Markdown. This lets it check
prose for disabled features and other platforms without compiling every feature
combination or evaluating the host's configuration.

## Why separate alternatives matter

Consider documentation with alternative code-fence openings:

````rust
/// Delivery examples.
#[cfg_attr(unix, doc = "```rust")]
#[cfg_attr(not(unix), doc = "```ignore")]
/// let transport = connect();
/// ```
///
/// Configure the delivery address before connecting.
pub struct Delivery;
````

The openings belong to different documentation versions. Concatenating both
would change Markdown's fence structure and could incorrectly spellcheck code
or hide surrounding prose. Instead, each selected version is assembled in source
order and passed through Markdown processing separately.

Repeated predicates stay correlated, nested `cfg_attr` conditions are combined,
and `cfg` constraints on enclosing owners are inherited. Sibling owners do not
inherit one another's pending attributes. Distinct source fragments remain
distinct even when they contain identical text.

## Symbolic examples, not build configurations

Diagnostics for conditional documentation include a symbolic example on the first
error line, with the `cfg(...)` annotation styled bold purple:

```text
error: spellcheck(Hunspell) with cfg(all(feature = "tls", not(feature = "trace")))
```

The example is one assignment selecting the reported documentation version. It
is not a claim that this configuration was compiled, that Cargo enables those
features together, or that the assignment corresponds to an actual Rust target.
Known platform relationships are enforced: `unix` is equivalent to
`target_family = "unix"`, and `windows` is equivalent to
`target_family = "windows"`. The Unix and Windows families are mutually exclusive,
but neither is required. Other family values can coexist: for example,
`target_family = "unix"` and `target_family = "wasm"` remain compatible. These
constraints are not complete target validation; satisfying them does not
guarantee that an actual Rust target exists.

Compiler-version predicates stay symbolic too. Equivalent release spellings,
such as `version("1.88")` and `version("1.88.0")`, share a threshold. Newer
thresholds imply older ones: `version("1.90")` cannot be selected together with
`not(version("1.88"))`. These relationships apply to both exact projections and
sampled fallback; the compiler running the checker is not used to choose a
branch.

The parser accepts supported attribute spellings across Rust editions, including
some feature-gated predicate forms. This does not enable those compiler features
or validate whether the input compiles under a particular edition or release.

## Coverage and resource limits

Binary decision diagrams partition configurations by selected source fragments,
not by enumerating every possible Boolean assignment. Many assignments can share
one documentation version.

The default per-owner `CfgBudget` limits are:

| Limit | Default |
| --- | ---: |
| Symbolic variables | 1,024 |
| Nodes produced by one BDD operation | 4,096 |
| Tasks explored by one BDD operation | 16,384 |
| Symbolic regions, including an empty selection | 1,024 |
| Complete fallback assignments sampled | 1,024 |

When a symbolic limit is exceeded, extraction reports incomplete coverage and
checks a bounded sample instead. Each retained sample evaluates all fragment
guards and owner constraints; documentation is not truncated to a processed
prefix. Sampling can miss reachable alternatives. Finding no valid sample does
not prove that the owner is unreachable. The limits are not a global memory or
runtime guarantee.

Nested cfg operators are limited to 32 levels. Extraction also preflights syntax
before invoking recursive parsers and limits structural traversal. Syntax beyond
those conservative safety limits is skipped with a source-spanned warning rather
than treated as fully checked. These syntax limits are separate from BDD
sampling budgets.

Library callers can set `CfgBudget` through
`Clusters::load_from_str_with_budget` or `Documentation::add_rust_with_budget`.
They can inspect `Clusters::warnings()` or `Documentation::warnings()` even if
an affected source has no checkable chunks.

When combining `Documentation` values, use `merge` to preserve both chunks and
warnings, or `merge_unique` to avoid repeating existing origins and identical
warnings. The chunk-only iterator APIs, including `extend`, do not transfer
structured warnings. Recursive source extraction uses the warning-preserving
merge path.

## Macros and safe text changes

Documentation macros, including `include_str!`, are not expanded. Unknown macro
content might open or close a Markdown fence, so a version selecting that content
is skipped with a warning. A complete alternative that does not select the macro
can still be checked. Unsupported attribute payloads are handled conservatively
as well; arbitrary macro-defined ownership is not inferred.

Source mappings retain original locations, including Unicode and gaps between
selected fragments. Identical diagnostics from shared prose are deduplicated by
source location. Conditional or structurally unsafe chunks are excluded from
reflow so that wrapping cannot overwrite intervening Rust syntax; ordinary safe
documentation remains reflowable.

The `doc-chunks` library declares Rust 1.88 as its minimum version;
`cargo-spellcheck` requires Rust 1.89.
