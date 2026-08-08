# strata-jsonc

JSONC ([jsonc.org](https://jsonc.org) "JSON with Comments") resolver for the Strata parser
ecosystem: a read-only, dynamic document model with owned values — decoded strings and converted
numbers — that keeps source fidelity available (spans, raw bytes, member order, duplicate members,
and trivia, with comments surfaced on the model), built on `strata-jsonc-parser`'s CST.

## Status

This crate is in initial development. The public API is not settled yet.

## Usage

```rust
use strata_jsonc::{JsoncError, JsoncNumber, JsoncResolver};

fn main() -> Result<(), JsoncError> {
    let resolver = JsoncResolver::builder().allow_trailing_commas(true).build();
    let document = resolver.resolve_str(
        "{\n  // the crate name\n  \"name\": \"caf\\u00e9\",\n  \"count\": 3,\n}",
    )?;

    let Some(object) = document.root().as_object() else {
        panic!("expected an object root");
    };

    // Owned, decoded values...
    assert_eq!(object.get("name").and_then(|v| v.as_str()), Some("café"));
    assert_eq!(
        object.get("count").and_then(|v| v.as_number()),
        Some(JsoncNumber::U64(3))
    );

    // ...with the comments surfaced on the model.
    let [name] = &object.get_all("name").collect::<Vec<_>>()[..] else {
        panic!("expected one \"name\" member");
    };
    let Some(comment) = name.name_meta().comments().next() else {
        panic!("expected the leading comment");
    };
    assert_eq!(
        document.comment_text(comment).as_deref(),
        Some(" the crate name")
    );

    Ok(())
}
```

See [docs/strata-jsonc.md](docs/strata-jsonc.md) for the design reference, including the
comment ergonomics and the duplicate-member and lone-surrogate policies.

## Minimum Supported Rust Version

The MSRV is **1.87.0**.

## License

Licensed under the Mozilla Public License, Version 2.0 (see [LICENSE](LICENSE)
or <https://www.mozilla.org/en-US/MPL/2.0/>).

MPL-2.0 is a file-level weak copyleft license: its obligations attach to this crate's files (in
source and compiled form) and to modifications of them, not to your own source files that merely
use this crate. Distributing a binary that includes this crate obliges you to tell recipients
that the crate is included and how to obtain its source; for an unmodified crate, pointing them
at the exact crates.io release is a reasonable way to do that. Example material (documentation
code examples, `examples/`, integration tests, and benchmarks) is additionally licensed under
[MIT-0](LICENSE-MIT-0), so it can be copied freely. See
[LICENSE-CLARIFICATION.md](LICENSE-CLARIFICATION.md) for the maintainers' full statement of
intent regarding normal Rust usage (linking, monomorphization, macro expansion), binary
distribution, and generated code, and [CONTRIBUTING.md](CONTRIBUTING.md) for the contribution
policy.
