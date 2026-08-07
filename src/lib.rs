//! JSONC (jsonc.org "JSON with Comments") parser for the Strata parser ecosystem.
//!
//! `strata-jsonc` is a strict [jsonc.org](https://jsonc.org) parser composed on top of
//! `strata-json`: JSONC extends RFC 8259 only with `//` line comments and non-nesting `/* */`
//! block comments, allowed anywhere insignificant whitespace is allowed. Comments are trivia —
//! retained in the CST with spans, never affecting the parsed data — and everything else stays
//! exactly as strict as `strata-json`: no `#` comments, no nested block comments, no `NaN`, no
//! single quotes, full string/number shape validation.
//!
//! The crate has three public layers, mirroring `strata-json`:
//!
//! - [`rules`] — `strata-json`'s RFC 8259 rules lifted into [`JsoncToken`], plus
//!   [`LineCommentRule`] and [`BlockCommentRule`]. All rules are exported individually so a
//!   further superset can compose on JSONC the same way.
//! - [`cst`] — the document shape: [`Document`], [`Value`], nodes, tokens, and trivia, with
//!   comments as [`TriviaKind::LineComment`]/[`TriviaKind::BlockComment`].
//! - [`parse`] — [`JsoncParser`], strict by default via [`JsoncParser::new`], with a builder for
//!   the spec-permitted options: `allow_trailing_commas` (jsonc.org Appendix A: parsers "MAY
//!   support trailing commas"), plus `allow_bom` and `max_depth` forwarded from `strata-json`.
//!
//! # Example
//!
//! ```
//! use strata_jsonc::{JsoncParser, TriviaKind, Value};
//!
//! # fn main() -> Result<(), strata_jsonc::JsoncError> {
//! let document = JsoncParser::new().parse_str(
//!     "{\n  // the crate name\n  \"name\": \"strata\"\n}",
//! )?;
//!
//! let Value::Object(object) = document.root() else {
//!     panic!("expected an object");
//! };
//! let member = object.members.first().unwrap();
//!
//! // The comment is retained as trivia on the token it precedes.
//! let comment = member.name.leading_comments().next().unwrap();
//! assert_eq!(comment.kind, TriviaKind::LineComment);
//! assert_eq!(
//!     document.trivia_bytes(comment),
//!     Some(&b"// the crate name"[..])
//! );
//! # Ok(())
//! # }
//! ```

pub mod cst;
pub mod parse;
pub mod rules;

pub use cst::{
    ArrayNode, Document, Element, Member, ObjectNode, SyntaxToken, Trivia, TriviaKind, Value,
};
pub use parse::{DEFAULT_MAX_DEPTH, JsoncError, JsoncErrorKind, JsoncParser, JsoncParserBuilder};
pub use rules::{
    BlockCommentRule, JsoncToken, LineCommentRule, StrictRuleSet, strict_rule_set, strict_scanner,
};

// Re-exported so downstream code can name the kinds carried inside `JsoncToken`/`SyntaxToken`
// and share rule sets without depending on `strata-json` directly. `SharedRuleSet` is
// kind-generic upstream, so it wraps JSONC rule sets too.
pub use strata_json::{JsonToken, SharedRuleSet};
