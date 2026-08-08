//! Resolver error types.

use std::error;
use std::fmt;
use strata_lexer::{Position, Span};

/// A resolve failure, carrying the position at which it was observed.
#[derive(Debug)]
pub struct JsoncError {
    /// Position of the offending byte or token start.
    pub position: Position,
    /// What went wrong.
    pub kind: JsoncErrorKind,
}

/// Categories of resolve failures.
#[derive(Debug)]
pub enum JsoncErrorKind {
    /// Parsing failed before resolution began.
    Parse(strata_jsonc_parser::JsoncError),
    /// A string contained a UTF-16 surrogate escape without its pair, under
    /// [`LoneSurrogates::Error`](strata_jsonc_parser::decode::LoneSurrogates::Error).
    LoneSurrogate {
        /// The unpaired code unit.
        unit: u16,
    },
    /// An object repeated a member name, under
    /// [`DuplicateKeys::Error`](crate::DuplicateKeys::Error).
    ///
    /// The position is the start of the repeated (later) member name.
    DuplicateMember {
        /// The decoded member name.
        name: String,
    },
    /// A scalar lexeme failed to decode.
    ///
    /// Tokens produced by the strict jsonc.org rules always decode; this is only reachable when
    /// a replaced rule set (via `with_rules`) admits token shapes the RFC 8259 grammar does not.
    InvalidLexeme {
        /// The decode failure, with its offset inside the lexeme.
        error: strata_jsonc_parser::decode::Error,
    },
    /// A span of the document was no longer backed by its retained source.
    ///
    /// This only happens when a caller reclaims the source store of a parsed document (e.g. via
    /// `source_mut().retire_all()`) before handing it to
    /// [`JsoncResolver::resolve_document`](crate::JsoncResolver::resolve_document).
    SourceUnavailable {
        /// The unavailable byte range.
        span: Span,
    },
}

impl fmt::Display for JsoncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            // The parse error's message already carries its position.
            JsoncErrorKind::Parse(error) => write!(formatter, "{error}"),
            JsoncErrorKind::LoneSurrogate { unit } => write!(
                formatter,
                "lone surrogate escape \\u{unit:04X} at line {}, column {} (byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            JsoncErrorKind::DuplicateMember { name } => write!(
                formatter,
                "duplicate object member {name:?} at line {}, column {} (byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            JsoncErrorKind::InvalidLexeme { error } => write!(
                formatter,
                "{error} (token at line {}, column {}, byte {})",
                self.position.line(),
                self.position.column(),
                self.position.byte()
            ),
            JsoncErrorKind::SourceUnavailable { span } => write!(
                formatter,
                "source bytes {}..{} are no longer retained",
                span.start(),
                span.end()
            ),
        }
    }
}

impl error::Error for JsoncError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            JsoncErrorKind::Parse(error) => Some(error),
            _ => None,
        }
    }
}

impl From<strata_jsonc_parser::JsoncError> for JsoncError {
    fn from(error: strata_jsonc_parser::JsoncError) -> Self {
        Self {
            position: error.position,
            kind: JsoncErrorKind::Parse(error),
        }
    }
}
