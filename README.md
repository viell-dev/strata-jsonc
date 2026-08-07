# strata-jsonc

JSONC ([jsonc.org](https://jsonc.org) "JSON with Comments") parser for the Strata parser ecosystem.

Composed on top of [`strata-json`](https://codeberg.org/Devious-Concepts/strata-json): JSONC adds
`//` line comments and non-nesting `/* */` block comments to RFC 8259, and nothing else. Comments
are retained in the lossless, zero-copy concrete syntax tree as trivia with exact spans; all of
`strata-json`'s strictness is kept. The parser is strict by default — the spec-permitted options
(`allow_trailing_commas` per jsonc.org Appendix A, `allow_bom` and `max_depth` per RFC 8259) are
off until enabled through the builder.

## Usage

```rust
use strata_jsonc::{JsoncParser, TriviaKind, Value};

fn main() -> Result<(), strata_jsonc::JsoncError> {
    let parser = JsoncParser::builder().allow_trailing_commas(true).build();
    let document = parser.parse_str(
        "{\n  // the crate name\n  \"name\": \"strata\",\n}",
    )?;

    let Value::Object(object) = document.root() else {
        panic!("expected an object");
    };
    let member = object.members.first().unwrap();

    // The comment is retained as trivia on the token it precedes, zero-copy.
    let comment = member.name.leading_comments().next().unwrap();
    assert_eq!(comment.kind, TriviaKind::LineComment);
    assert_eq!(document.trivia_bytes(comment), Some(&b"// the crate name"[..]));

    // The tolerated trailing comma is retained too.
    assert!(member.comma.is_some());
    Ok(())
}
```

See [docs/strata-jsonc.md](docs/strata-jsonc.md) for the API map, the token/CST shape, and the
composition points for further superset languages.

## Status

This crate is in initial development. The public API is not settled yet.

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
