# strata-jsonc

`strata-jsonc` is the JSONC ([jsonc.org](https://jsonc.org) "JSON with Comments") resolver for
the Strata parser ecosystem — the consumer-facing crate of the JSONC stack. It drives
`strata-jsonc-parser` and turns the zero-copy CST into a read-only, owned document model:
strings unescaped, numbers converted integer-preserving, while source fidelity stays available
on every node (byte spans, positions, leading trivia — including the comments that make JSONC —
member order, duplicate members) and the raw source text stays reachable through the document.
Comments are surfaced on the model: `Meta::comments()` iterates a node's leading comments and
`JsoncDocument::comment_text` extracts a comment's content with its delimiters stripped.

The crate sits at the resolver stage of the per-language stack:

```text
strata-jsonc-rules -> strata-jsonc-parser -> strata-jsonc (this crate)
```

- **Edition / MSRV:** Rust 2024, MSRV `1.87.0`.
- **License:** MPL-2.0.
- **Dependencies:** `strata-jsonc-parser`, `strata-lexer`.

## 1. Module and public-API map

| Module     | Public items |
|------------|--------------|
| `resolve`  | `JsoncResolver<M>`, `JsoncResolverBuilder<M>`, `DuplicateKeys` |
| `value`    | `JsoncValue`, `JsoncArray`, `JsoncObject`, `JsoncMember`, `JsoncNumber`, `NumberFormat`, `Meta` |
| `document` | `JsoncDocument` |
| `error`    | `JsoncError`, `JsoncErrorKind` |

Everything is re-exported at the crate root, plus `LoneSurrogates` and the parser's
`Trivia`/`TriviaKind`.

## 2. The resolver (`resolve`)

`JsoncResolver::new()` / `default()` is the fully strict path: strict parser defaults (comments
accepted and retained — they are core JSONC, not an option), duplicates retained, lone
surrogates rejected. `resolve(impl Read)`, `resolve_str`, and `resolve_bytes` parse and resolve
in one step; `resolve_document(parser::Document)` resolves an already-parsed CST (whose retained
source must still back the tree — a reclaimed store yields `SourceUnavailable`).

`JsoncResolver::builder()` exposes the parser passthroughs (`allow_bom`,
`allow_trailing_commas` — the jsonc.org Appendix A opt-in — `max_depth`, `with_rules`,
`with_lexer_config`) plus the two resolution policies:

- `duplicate_keys(DuplicateKeys)` — `Retain` (default) keeps every member; `Error` rejects the
  document at the repeated name with the decoded name in the error.
- `lone_surrogates(LoneSurrogates)` — `Error` (default) rejects `"\uD800"`-style unpaired
  escapes at the offending backslash; `Replace` substitutes U+FFFD.

## 3. The model (`value`, `document`)

- `JsoncValue` — `Null(Meta)`, `Bool(bool, Meta)`, `Number(JsoncNumber, NumberFormat, Meta)`,
  `String(String, Meta)`, `Array(JsoncArray)`, `Object(JsoncObject)`, with `meta()`, `span()`,
  and `as_*`/`is_*` accessors. `JsoncNumber` is the parser `decode::Number` re-exported: `U64`
  for non-negative integers that fit, `I64` for negative, `F64` otherwise (`-0` keeps its sign;
  oversized integers round with the raw text still reachable for arbitrary precision). The written form rides along as a `NumberFormat` (`Integer`, `Decimal`, `Exponential`) exposed by `JsoncValue::number_format()`.
- `Meta` — per-node fidelity: lexeme span (containers cover open through close), start/end
  positions, and the leading trivia (whitespace runs with `line_breaks()`/`blank_lines()`, a
  tolerated BOM, and comments). Metas are only created by resolution, so they always describe
  genuine nodes.
- **Comment ergonomics.** `Meta::comments()` filters the leading trivia down to the comment
  pieces, in source order — a comment written before a member arrives on the member name's
  `Meta`. `JsoncDocument::comment_text(&Trivia)` extracts the content: line comments lose their
  `//` prefix (their span never includes the terminating line break), block comments lose both
  `/*` and `*/`, interior whitespace is kept verbatim. It returns `None` for non-comment pieces,
  for spans no longer backed by the retained source, and for non-UTF-8 bytes (unreachable under
  the strict rules, which validate comment encoding — only replaced rule sets can get there).
  Comments after the root value live in `JsoncDocument::trailing_trivia()`.
- `JsoncObject` — members in source order, duplicates retained. `get(name)` resolves to the
  **last** occurrence (ECMA-262 `JSON.parse` behavior); `get_all`, `members`, and
  `duplicates()` (members shadowed by a later same-name member) expose full fidelity.
  `JsoncMember` carries the decoded name, the name token's `Meta` (raw spelling reachable via
  its span), and the value.
- `JsoncDocument` — owns the root, trailing trivia, and the retained `SourceStore`:
  `bytes(span)` borrows raw source zero-copy, `source_ref(span)` hands out promotable handles
  that survive store reclamation, `source_mut()` allows owner-side reclamation (owned values are
  unaffected; only raw-byte lookups — `bytes` and thus `comment_text` — stop resolving).

## 4. Errors

`JsoncError { position, kind }` with `JsoncErrorKind`: `Parse(parser error)` (lexical and
grammar failures pass through, unterminated block comments and rejected `#` included),
`LoneSurrogate { unit }`, `DuplicateMember { name }` (at the repeated name),
`InvalidLexeme { error }` (decode failure — only reachable with replaced rule sets whose tokens
deviate from the RFC shapes), and `SourceUnavailable { span }` (resolving a document whose store
was reclaimed first). Decode-failure positions advance on the token's start line, which is exact
for strict-rule tokens because RFC lexemes cannot contain raw line breaks.

## 5. Design choices and open decisions

- **Read-only model.** Editing and fidelity-preserving write-back stay at the CST/source level
  of the lower layers; the resolver is the "give me real values" endpoint.
- **Comments surfaced, not modeled.** Comments stay the parser's `Trivia` values reached through
  `Meta`, and `comment_text` is a document lookup rather than a `String` stored per comment: the
  model stays lean, nothing is decoded that is not asked for, and reclamation semantics stay
  uniform with the other raw-byte accessors.
- **Last-occurrence `get`** matches ECMA-262 `JSON.parse` and most implementations. Flagged as a
  maintainer-unratified decision; `get_all`/`duplicates` keep every alternative reachable.
- **Trivia reuse.** `Meta` carries the parser's `Trivia` values directly instead of mirroring
  them into a resolver-local type — they are already owned, `Copy`, and construction-validated.
- **`-0` and hex-overflow classification** (via the shared decode module) are likewise
  documented-but-unratified: `-0` stays `F64(-0.0)`; integers beyond 64 bits round to `f64`.

## 6. Quality bar

Same as the ecosystem: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D
warnings` under the strict lint set in `Cargo.toml`, and `cargo test` all clean. Lint exceptions
use `#[expect(lint, reason = "…")]` on the smallest existing scope.

## 7. Limitations and follow-up

- No serde glue: deserialization onto user structs is a planned separate crate, which goes
  parser → structs directly rather than through this model.
- `JsoncObject::get` is a linear scan; a lazily built index is possible future work if large
  objects show up in practice.
- `comment_text` allocates a `String` per call; a borrowing variant could be added if comment
  processing shows up hot.
- The public API is not settled; this is an initial-development crate on an experimental branch.
- Package verification (`cargo package`) is blocked until the path dependencies are published.
