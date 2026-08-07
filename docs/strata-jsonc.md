# strata-jsonc

`strata-jsonc` is a strict [jsonc.org](https://jsonc.org) "JSON with Comments" parser for the
Strata parser ecosystem, composed on top of `strata-json`. JSONC extends RFC 8259 only with `//`
line comments and non-nesting `/* */` block comments, allowed anywhere insignificant whitespace
is allowed; comments are trivia — retained in the CST with spans, never affecting the parsed data
— and everything else stays exactly as strict as `strata-json` (no `#` comments, no `NaN`, no
single quotes, full string/number shape validation).

The crate sits at the parser stage of the ecosystem pipeline:

```text
reader -> lexer -> parser (this crate) -> resolver -> deserializer
```

- **Edition / MSRV:** Rust 2024, MSRV `1.87.0`.
- **License:** MPL-2.0.
- **Dependencies:** `strata-json` and `strata-lexer` (which bring `strata-reader` and
  `strata-lease`).

## 1. Module and public-API map

| Module  | Public items |
|---------|--------------|
| `rules` | `JsoncToken`, `LineCommentRule`, `BlockCommentRule`, `StrictRuleSet`, `strict_rule_set()`, `strict_scanner()` |
| `cst`   | `Document`, `Value`, `ArrayNode`, `Element`, `ObjectNode`, `Member`, `SyntaxToken`, `Trivia`, `TriviaKind` |
| `parse` | `JsoncParser<M>`, `JsoncParserBuilder<M>`, `JsoncError`, `JsoncErrorKind`, `DEFAULT_MAX_DEPTH` |

Everything is re-exported at the crate root, alongside `strata_json::JsonToken` and
`strata_json::SharedRuleSet` (kind-generic, so it wraps JSONC rule sets too).

## 2. Token rules (`rules`)

`JsoncToken` wraps `strata-json`'s kind rather than duplicating it:
`Json(JsonToken)` carries every shared shape, and `LineComment`, `BlockComment`, and `Eof` are
the only additions. Both comment kinds are trivia (`is_trivia()`); `describe()` yields diagnostic
names.

The reused JSON rules are lifted with
[`MatcherExt::map_kind`](https://codeberg.org/Devious-Concepts/strata-lexer): `StrictRuleSet`
holds the six strict RFC 8259 rules as `MapMatcher<Rule, fn(JsonToken) -> JsoncToken>` plus the
two comment rules, and implements `MatcherSet` by chaining both groups — fully monomorphic,
`Copy`, no boxing. Reused rules keep their validation: `01` is still a lexical error inside
JSONC. `BomRule` stays in the set; the parser owns the BOM policy, exactly as upstream.

The added rules:

- `LineCommentRule` — `//` up to (excluding) the next CR/LF, or to end of input. jsonc.org:
  "Single-line comments start with `//` and continue until a line ending is encountered." The
  line ending itself lexes as ordinary whitespace, keeping blank-line counting uniform.
- `BlockCommentRule` — `/*` to the *first* `*/` (jsonc.org: "Multi-line comments cannot be
  nested. The closing of the nested comment will be interpreted as the end of the outer
  comment."). The span covers both delimiters; `/*/` cannot self-close. An unclosed comment is
  `LexErrorKind::UnterminatedDelimiter` at the opening `/*`.

Both rules validate that comment content is UTF-8 (the input must be UTF-8 per RFC 8259 §8.1),
reporting the offending byte with a line/column-correct position even inside multiline comments.
Control characters are valid comment content per the jsonc.org ABNF. `#` is matched by no rule
("Single-line comments that start with `#` are not supported") and surfaces as an unrecognized
byte.

## 3. CST (`cst`)

The tree mirrors `strata-json`'s CST instead of reusing it: upstream's `TriviaKind` is a closed
enum (whitespace and BOM only) and its `Document` is not externally constructible, so a
comment-retaining tree needs its own thin layer. The shape is deliberately identical, with two
differences:

- `TriviaKind` gains `LineComment` and `BlockComment` (plus `is_comment()`, and
  `SyntaxToken::leading_comments()` / `Document::trivia_bytes()` helpers). Comments are trivia
  because the spec requires that "Removing all comments MUST yield the same data representation
  of the JSON values" — structurally, a comment can never reach the grammar.
- Significant tokens carry plain `JsonToken` kinds: JSONC adds no significant token kinds over
  RFC 8259, only trivia.

Everything else matches upstream: leading-trivia attachment (every trivia piece precedes exactly
one significant token, document-trailing trivia lives in `Document::trailing_trivia()`),
zero-copy spans into the document-owned `SourceStore`, `Trivia::line_breaks()`/`blank_lines()`,
duplicate members retained verbatim, and byte-for-byte source reconstruction
(`tests/fidelity.rs` proves it, comments and tolerated trailing commas included).

## 4. Parser (`parse`)

`JsoncParser::new()` / `default()` is the fully strict path: comments accepted and retained
(they are core JSONC, not an option), everything else strict — trailing commas rejected, no BOM
tolerance, `DEFAULT_MAX_DEPTH` (= `strata_json::DEFAULT_MAX_DEPTH`, 128) nesting. `parse(impl
Read)`, `parse_str`, and `parse_bytes` each run one document to EOF; trailing content other than
whitespace and comments is an error. The parser clones its rule set into a fresh `SpecScanner`
per parse, so one parser serves many inputs.

`JsoncParser::builder()` exposes:

- `allow_trailing_commas(bool)` — off by default. The spec's single optionality clause,
  jsonc.org Appendix A: "JSONC parsers MAY support trailing commas" (the grammar itself notes
  trailing commas are NOT supported, and the reference `jsonc-parser` leaves
  `allowTrailingComma` off unless configured). When on, a comma directly before `]`/`}` is
  tolerated and retained on the last `Element`/`Member` (fidelity); a lone comma (`[,]`, `{,}`)
  or repeated commas (`[1,,2]`) remain errors either way.
- `allow_bom(bool)` — off by default, forwarded from `strata-json` with identical semantics
  (RFC 8259 §8.1: implementations "MAY ignore the presence of a byte order mark"). Retained as
  `TriviaKind::Bom` when on, `JsoncErrorKind::UnexpectedBom` when off.
- `max_depth(usize)` — default 128, forwarded from `strata-json` (RFC 8259 §9 implementation
  limit). Comments do not consume depth.
- `with_rules(set)` — replace the token rule set (any `MatcherSet<Kind = JsoncToken> + Clone`;
  wrap non-`Clone` sets in `SharedRuleSet`).
- `with_lexer_config(config)` — replace the lexer/reader configuration; `compact_after` is
  always overridden to `None` at parse time.

Errors are `JsoncError { position, kind }` with one-based line/column plus absolute byte,
mirroring `JsonError`. `JsoncErrorKind`: `Lex(LexError)` (including unterminated block comments
at the `/*` and `#` as an unrecognized byte), `Expected { expected, found }`,
`UnexpectedEof { expected }`, `TrailingContent { found }`, `UnexpectedBom`,
`DepthLimitExceeded { limit }`. `found` kinds are plain `JsonToken`s — comments never reach the
grammar.

## 5. Composition points

This crate is itself the worked example of `strata-json` §5 composition: it defines its own kind
enum and parser and reuses the upstream *rules layer* only (no forking, no wrapping of
`JsonParser`). The same chain continues upward — a further superset composes on JSONC by lifting
the rules it keeps:

```rust
enum SuperToken { Jsonc(JsoncToken), HashComment, Eof }

let rules: MatcherVec<SuperToken> = vec![
    HashCommentRule.boxed(),                                          // the new shape
    LineCommentRule.map_kind(SuperToken::Jsonc).boxed(),              // JSONC's additions
    BlockCommentRule.map_kind(SuperToken::Jsonc).boxed(),
    WhitespaceRule.map_kind(|k| SuperToken::Jsonc(JsoncToken::Json(k))).boxed(), // JSON's rules
    // ...
];
```

`tests/composition.rs` is the worked example. Spec-permitted extensions follow the ecosystem
pattern: parser-level options, strict by default — `allow_trailing_commas` needs no extra rule
(`,` is already a token; the option is purely grammatical).

## 6. Design choices

- **Comments as trivia, enforced structurally.** The trivia fold in the grammar driver is the
  one place comments exist; the grammar itself never sees them, which *is* the spec's "comments
  are ignored during parsing" requirement.
- **Mirrored, not forked, CST.** Reuse of upstream CST types was evaluated and rejected on two
  hard blockers (closed `TriviaKind`, `pub(crate)` `Document::new`); the mirror keeps the shape
  identical so a future upstream generalization could collapse the two.
- **`JsonToken` in the tree.** Significant-token kinds stay upstream's enum, making "JSONC's
  data language is exactly JSON" visible in the types.
- **Validation at the lexical boundary.** Comment termination and UTF-8 are rule concerns with
  exact positions, like upstream's string/number shape checks.

## 7. Quality bar

Same as the ecosystem: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D
warnings` under the strict lint set in `Cargo.toml`, and `cargo test` all clean. Lint exceptions
use `#[expect(lint, reason = "…")]` on the smallest existing scope.

## 8. Limitations and follow-up

- No resolver: comment text extraction beyond raw bytes, string unescaping, and number
  conversion stay downstream.
- The CST mirror duplicates upstream's node types; if `strata-json` ever parameterizes its CST
  over the trivia kind, this crate should collapse onto it.
- `cargo package` verification is blocked until `strata-json` is published and the path
  dependency gains a version.
- The public API is not settled; this is an initial-development crate on an experimental branch.
