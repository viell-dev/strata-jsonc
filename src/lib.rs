//! JSONC (jsonc.org "JSON with Comments") resolver for the Strata parser ecosystem: a dynamic,
//! owned document model with comments and source fidelity.
//!
//! The resolver is the consumer-facing layer of the JSONC stack: it drives
//! `strata-jsonc-parser`, then turns the zero-copy CST into owned values — strings unescaped
//! (with UTF-16 surrogate-pair combination and a configurable lone-surrogate policy), numbers
//! converted integer-preserving — while keeping fidelity available on every node: byte spans,
//! positions, leading trivia (whitespace runs, blank lines, a tolerated BOM, and the comments
//! that make JSONC), member order, and duplicate members. Comments surface on the model:
//! [`Meta::comments`] iterates a node's leading comments and
//! [`JsoncDocument::comment_text`] extracts a comment's content with its delimiters stripped.
//! The raw source text behind any node stays reachable through [`JsoncDocument::bytes`] and
//! promotable [`source_ref`](JsoncDocument::source_ref) handles.
//!
//! [`JsoncResolver::new`] is the fully strict path; [`JsoncResolver::builder`] exposes the
//! parser's spec-permitted options (`allow_bom`, `max_depth`, and the opt-in
//! `allow_trailing_commas` jsonc.org Appendix A grants), lower-layer replacement, and the two
//! resolution policies: [`DuplicateKeys`] (retain by default, `get` resolves to the last
//! occurrence) and [`LoneSurrogates`] (reject by default). The model is read-only; editing and
//! writing back stay at the CST/source level of the lower layers.

mod document;
mod error;
mod resolve;
mod value;

pub use document::JsoncDocument;
pub use error::{JsoncError, JsoncErrorKind};
pub use resolve::{DuplicateKeys, JsoncResolver, JsoncResolverBuilder};
pub use strata_jsonc_parser::decode::LoneSurrogates;
pub use strata_jsonc_parser::{Trivia, TriviaKind};
pub use value::{JsoncArray, JsoncMember, JsoncNumber, JsoncObject, JsoncValue, Meta};
