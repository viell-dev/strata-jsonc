//! The strict jsonc.org parser: token stream in, CST out.
//!
//! [`JsoncParser::new`] is the fully strict path: comments are accepted (they are core JSONC,
//! not an option) and retained as trivia; everything else is exactly RFC 8259 as `strata-json`
//! enforces it. [`JsoncParser::builder`] exposes the spec-permitted options — the trailing-comma
//! tolerance jsonc.org grants ("JSONC parsers MAY support trailing commas"), plus the two RFC
//! 8259 latitudes forwarded from `strata-json` (`allow_bom`, `max_depth`) — and `with_*` hooks
//! to customize or replace the lower layers: the token rules and the lexer configuration.

use crate::cst::{
    ArrayNode, Document, Element, Member, ObjectNode, SyntaxToken, Trivia, TriviaKind, Value,
};
use crate::rules::{JsoncToken, StrictRuleSet, strict_rule_set};
use std::error::Error;
use std::fmt;
use std::io::{Cursor, Read};
use strata_json::JsonToken;
use strata_lexer::{
    LexError, Lexer, LexerConfig, MatcherSet, Position, SourceStore, SpecScanner, Token,
};

/// The default nesting-depth limit, identical to `strata-json`'s.
///
/// RFC 8259 §9 permits parsers to set limits on the depth of nesting; this is the maximum number
/// of nested arrays/objects. Override it with [`JsoncParserBuilder::max_depth`]. The limit also
/// bounds the parser's recursion, so raising it substantially trades stack headroom for depth.
pub const DEFAULT_MAX_DEPTH: usize = strata_json::DEFAULT_MAX_DEPTH;

/// A parse failure, carrying the position at which it was observed.
#[derive(Debug)]
pub struct JsoncError {
    /// Position of the offending byte or token start.
    pub position: Position,
    /// What went wrong.
    pub kind: JsoncErrorKind,
}

/// Categories of parse failures.
///
/// `found` kinds are plain [`JsonToken`]s: comments are trivia and never reach the grammar, so
/// every significant token the grammar can trip over is a JSON kind.
#[derive(Debug)]
pub enum JsoncErrorKind {
    /// The lexer failed: I/O, an unrecognized byte (e.g. a `#` — not a JSONC comment), a
    /// token-shape violation reported at the offending byte, or an unterminated block comment
    /// reported at its opening `/*`.
    Lex(LexError),
    /// A token of an unexpected kind where the grammar required something else.
    Expected {
        /// What the grammar required here.
        expected: &'static str,
        /// The token kind actually found.
        found: JsonToken,
    },
    /// The input ended where the grammar required more.
    UnexpectedEof {
        /// What the grammar required here.
        expected: &'static str,
    },
    /// A significant token followed the complete top-level value.
    TrailingContent {
        /// The token kind found after the value.
        found: JsonToken,
    },
    /// The input starts with a byte-order mark and the parser was not configured to tolerate it.
    ///
    /// RFC 8259 §8.1 forbids adding a BOM but permits implementations to ignore one; enable
    /// [`JsoncParserBuilder::allow_bom`] to opt into that permission.
    UnexpectedBom,
    /// Nesting exceeded the configured depth limit (RFC 8259 §9 implementation limit).
    DepthLimitExceeded {
        /// The configured limit.
        limit: usize,
    },
}

impl fmt::Display for JsoncErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lex(error) => write!(formatter, "{error}"),
            Self::Expected { expected, found } => {
                write!(formatter, "expected {expected}, found {}", found.describe())
            }
            Self::UnexpectedEof { expected } => {
                write!(formatter, "unexpected end of input, expected {expected}")
            }
            Self::TrailingContent { found } => write!(
                formatter,
                "unexpected {} after the top-level value",
                found.describe()
            ),
            Self::UnexpectedBom => formatter.write_str(
                "leading byte-order mark (enable allow_bom to tolerate it per RFC 8259 \u{a7}8.1)",
            ),
            Self::DepthLimitExceeded { limit } => {
                write!(formatter, "nesting depth limit of {limit} exceeded")
            }
        }
    }
}

impl fmt::Display for JsoncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            // The lexer's message already carries its byte position.
            JsoncErrorKind::Lex(error) => write!(formatter, "{error}"),
            kind => write!(
                formatter,
                "{kind} at line {}, column {} (byte {})",
                self.position.line, self.position.column, self.position.byte
            ),
        }
    }
}

impl Error for JsoncError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            JsoncErrorKind::Lex(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LexError> for JsoncError {
    fn from(error: LexError) -> Self {
        Self {
            position: error.position,
            kind: JsoncErrorKind::Lex(error),
        }
    }
}

/// A reusable, strict jsonc.org parser.
///
/// `M` is the token rule set handed to the lexer for every parse; it defaults to the strict
/// jsonc.org rules. The parser clones the rules into a fresh scanner per [`JsoncParser::parse`]
/// call, so one configured parser can serve many inputs.
#[derive(Debug, Clone)]
pub struct JsoncParser<M = StrictRuleSet> {
    rules: M,
    allow_bom: bool,
    allow_trailing_commas: bool,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl JsoncParser<StrictRuleSet> {
    /// Creates a fully strict parser: comments accepted and retained (core JSONC), trailing
    /// commas rejected, no BOM tolerance, [`DEFAULT_MAX_DEPTH`] nesting.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            allow_bom: false,
            allow_trailing_commas: false,
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }

    /// Starts a builder for customizing spec-permitted options or replacing lower layers.
    pub fn builder() -> JsoncParserBuilder<StrictRuleSet> {
        JsoncParserBuilder::new()
    }
}

impl Default for JsoncParser<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> JsoncParser<M>
where
    M: MatcherSet<Kind = JsoncToken> + Clone,
{
    /// Parses a complete JSONC text from `reader` into a [`Document`].
    ///
    /// Any RFC 8259 value is a valid top-level document. The entire input must be consumed:
    /// trailing content other than whitespace and comments is an error. On success the consumed
    /// source bytes are retained in the document's [`SourceStore`], keeping the CST zero-copy.
    pub fn parse<R: Read>(&self, reader: R) -> Result<Document, JsoncError> {
        // Compaction must stay off so the consumed bytes survive until they are parked into the
        // document's source store.
        let config = self.lexer_config.with_compact_after(None);
        let scanner = SpecScanner::new(self.rules.clone(), JsoncToken::Eof);
        let mut driver = Driver {
            lexer: Lexer::with_config(reader, scanner, config),
            pending: Vec::new(),
            peeked: None,
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
            max_depth: self.max_depth,
        };

        let root = driver.parse_value(0)?;
        let eof = driver.next_token()?;

        if eof.kind != JsonToken::Eof {
            return Err(JsoncError {
                position: eof.start,
                kind: JsoncErrorKind::TrailingContent { found: eof.kind },
            });
        }

        let (mut input, _scanner) = driver.lexer.into_parts();
        let mut source = SourceStore::new();
        input.park_consumed_buffer(&mut source)?;

        Ok(Document::new(root, eof.leading, source))
    }

    /// Parses a complete JSONC text from a string slice.
    pub fn parse_str(&self, input: &str) -> Result<Document, JsoncError> {
        self.parse(Cursor::new(input.as_bytes()))
    }

    /// Parses a complete JSONC text from a byte slice. The bytes must be UTF-8 (RFC 8259 §8.1);
    /// violations inside strings or comments are lexical errors, and any non-ASCII byte outside
    /// them matches no rule.
    pub fn parse_bytes(&self, input: &[u8]) -> Result<Document, JsoncError> {
        self.parse(Cursor::new(input))
    }

    /// The token rule set handed to the lexer at each parse.
    pub const fn rules(&self) -> &M {
        &self.rules
    }

    /// Whether a leading byte-order mark is tolerated (RFC 8259 §8.1).
    pub const fn allow_bom(&self) -> bool {
        self.allow_bom
    }

    /// Whether a trailing comma before `]`/`}` is tolerated (jsonc.org Appendix A).
    pub const fn allow_trailing_commas(&self) -> bool {
        self.allow_trailing_commas
    }

    /// The configured nesting-depth limit (RFC 8259 §9).
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }
}

/// Builder for [`JsoncParser`], strict by default.
#[derive(Debug, Clone)]
pub struct JsoncParserBuilder<M = StrictRuleSet> {
    rules: M,
    allow_bom: bool,
    allow_trailing_commas: bool,
    max_depth: usize,
    lexer_config: LexerConfig,
}

impl JsoncParserBuilder<StrictRuleSet> {
    /// Starts from the fully strict defaults.
    pub fn new() -> Self {
        Self {
            rules: strict_rule_set(),
            allow_bom: false,
            allow_trailing_commas: false,
            max_depth: DEFAULT_MAX_DEPTH,
            lexer_config: LexerConfig::new(),
        }
    }
}

impl Default for JsoncParserBuilder<StrictRuleSet> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> JsoncParserBuilder<M> {
    /// Replaces the token rule set handed to the lexer.
    ///
    /// This is the rules-layer composition point: tests can mock the rules, and adjusted rule
    /// sets can be swapped in without touching the grammar driver. The set must still produce
    /// [`JsoncToken`] kinds; a superset *language* instead defines its own kind enum and parser,
    /// reusing this crate's rule structs via
    /// [`MatcherExt::map_kind`](strata_lexer::MatcherExt::map_kind).
    pub fn with_rules<N>(self, rules: N) -> JsoncParserBuilder<N>
    where
        N: MatcherSet<Kind = JsoncToken> + Clone,
    {
        JsoncParserBuilder {
            rules,
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }

    /// Replaces the lexer configuration (reader capacities, position tracking, ...).
    ///
    /// The parser always disables buffer compaction (`compact_after = None`) at parse time: the
    /// consumed bytes must survive until they are parked into the document's source store.
    #[must_use]
    pub const fn with_lexer_config(mut self, config: LexerConfig) -> Self {
        self.lexer_config = config;
        self
    }

    /// Tolerates one leading byte-order mark, retaining it as trivia.
    ///
    /// Off by default; forwarded from `strata-json` with an identical default. RFC 8259 §8.1: a
    /// JSON text must not begin with a BOM, but implementations "MAY ignore the presence of a
    /// byte order mark rather than treating it as an error". With the option off, a leading BOM
    /// is [`JsoncErrorKind::UnexpectedBom`]; a BOM anywhere else in the input is always an error.
    #[must_use]
    pub const fn allow_bom(mut self, allow: bool) -> Self {
        self.allow_bom = allow;
        self
    }

    /// Tolerates a trailing comma before `]` and `}`, retaining it in the CST.
    ///
    /// Off by default. jsonc.org grammar: "trailing commas are NOT supported"; Appendix A:
    /// "JSONC parsers MAY support trailing commas" — the reference `jsonc-parser` likewise
    /// leaves `allowTrailingComma` off unless configured. When enabled, the tolerated comma is
    /// kept on the last [`Element`]/[`Member`] for source fidelity. A comma with no preceding
    /// entry (`[,]`, `{,}`) or a repeated comma (`[1,,2]`) remains an error either way.
    #[must_use]
    pub const fn allow_trailing_commas(mut self, allow: bool) -> Self {
        self.allow_trailing_commas = allow;
        self
    }

    /// Sets the nesting-depth limit.
    ///
    /// Forwarded from `strata-json` with an identical default. RFC 8259 §9: "An implementation
    /// may set limits on the maximum depth of nesting." The limit counts nested arrays/objects —
    /// with a limit of 1, `[1]` parses and `[[1]]` does not. It also bounds the parser's
    /// recursion depth; see [`DEFAULT_MAX_DEPTH`].
    #[must_use]
    pub const fn max_depth(mut self, limit: usize) -> Self {
        self.max_depth = limit;
        self
    }

    /// Finishes the builder.
    pub fn build(self) -> JsoncParser<M>
    where
        M: MatcherSet<Kind = JsoncToken> + Clone,
    {
        JsoncParser {
            rules: self.rules,
            allow_bom: self.allow_bom,
            allow_trailing_commas: self.allow_trailing_commas,
            max_depth: self.max_depth,
            lexer_config: self.lexer_config,
        }
    }
}

/// The per-parse grammar driver: pulls tokens, folds trivia (whitespace, BOM, comments) into the
/// next significant token, and recursively builds the CST.
struct Driver<R: Read, M: MatcherSet<Kind = JsoncToken>> {
    lexer: Lexer<R, SpecScanner<M, JsoncToken>>,
    pending: Vec<Trivia>,
    peeked: Option<SyntaxToken>,
    allow_bom: bool,
    allow_trailing_commas: bool,
    max_depth: usize,
}

impl<R: Read, M: MatcherSet<Kind = JsoncToken>> Driver<R, M> {
    /// Returns the next significant token with its leading trivia attached.
    ///
    /// Comments are trivia here — this is the point where JSONC's "comments must not affect the
    /// parsed data" requirement is enforced structurally: a comment can never reach the grammar.
    fn next_token(&mut self) -> Result<SyntaxToken, JsoncError> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }

        loop {
            let token = self.lexer.next_token()?;

            match token.kind {
                JsoncToken::Json(JsonToken::Whitespace) => {
                    self.push_trivia(TriviaKind::Whitespace, &token);
                }
                JsoncToken::LineComment => self.push_trivia(TriviaKind::LineComment, &token),
                JsoncToken::BlockComment => self.push_trivia(TriviaKind::BlockComment, &token),
                JsoncToken::Json(JsonToken::Bom) => {
                    if self.allow_bom {
                        self.push_trivia(TriviaKind::Bom, &token);
                    } else {
                        return Err(JsoncError {
                            position: token.start,
                            kind: JsoncErrorKind::UnexpectedBom,
                        });
                    }
                }
                JsoncToken::Json(kind) => {
                    return Ok(self.significant(kind, &token));
                }
                JsoncToken::Eof => {
                    return Ok(self.significant(JsonToken::Eof, &token));
                }
            }
        }
    }

    /// Builds a significant [`SyntaxToken`], attaching the pending trivia.
    fn significant(&mut self, kind: JsonToken, token: &Token<JsoncToken>) -> SyntaxToken {
        SyntaxToken {
            kind,
            span: token.span,
            start: token.start,
            end: token.end,
            leading: std::mem::take(&mut self.pending),
        }
    }

    /// Peeks the kind of the next significant token without consuming it.
    fn peek_kind(&mut self) -> Result<JsonToken, JsoncError> {
        if self.peeked.is_none() {
            self.peeked = Some(self.next_token()?);
        }

        Ok(self
            .peeked
            .as_ref()
            .map_or(JsonToken::Eof, |token| token.kind))
    }

    fn push_trivia(&mut self, kind: TriviaKind, token: &Token<JsoncToken>) {
        self.pending.push(Trivia {
            kind,
            span: token.span,
            start: token.start,
            end: token.end,
        });
    }

    /// Parses one value. `depth` is the number of containers already entered.
    fn parse_value(&mut self, depth: usize) -> Result<Value, JsoncError> {
        let token = self.next_token()?;

        match token.kind {
            JsonToken::LeftBrace => self.parse_object(token, depth),
            JsonToken::LeftBracket => self.parse_array(token, depth),
            JsonToken::String => Ok(Value::String(token)),
            JsonToken::Number => Ok(Value::Number(token)),
            JsonToken::True => Ok(Value::Bool { value: true, token }),
            JsonToken::False => Ok(Value::Bool {
                value: false,
                token,
            }),
            JsonToken::Null => Ok(Value::Null(token)),
            JsonToken::Eof => Err(JsoncError {
                position: token.start,
                kind: JsoncErrorKind::UnexpectedEof {
                    expected: "a JSON value",
                },
            }),
            found => Err(JsoncError {
                position: token.start,
                kind: JsoncErrorKind::Expected {
                    expected: "a JSON value",
                    found,
                },
            }),
        }
    }

    /// Checks the depth limit on entering a container, returning the child depth.
    fn enter_container(&self, depth: usize, open: &SyntaxToken) -> Result<usize, JsoncError> {
        let entered = depth.saturating_add(1);

        if entered > self.max_depth {
            Err(JsoncError {
                position: open.start,
                kind: JsoncErrorKind::DepthLimitExceeded {
                    limit: self.max_depth,
                },
            })
        } else {
            Ok(entered)
        }
    }

    fn parse_object(&mut self, open: SyntaxToken, depth: usize) -> Result<Value, JsoncError> {
        let depth = self.enter_container(depth, &open)?;
        let mut members = Vec::new();

        if self.peek_kind()? == JsonToken::RightBrace {
            let close = self.next_token()?;
            return Ok(Value::Object(ObjectNode {
                open,
                members,
                close,
            }));
        }

        loop {
            let name = self.next_token()?;

            match name.kind {
                JsonToken::String => {}
                JsonToken::Eof => {
                    return Err(JsoncError {
                        position: name.start,
                        kind: JsoncErrorKind::UnexpectedEof {
                            expected: "an object member name (a string)",
                        },
                    });
                }
                found => {
                    return Err(JsoncError {
                        position: name.start,
                        kind: JsoncErrorKind::Expected {
                            expected: "an object member name (a string)",
                            found,
                        },
                    });
                }
            }

            let colon = self.expect(JsonToken::Colon, "':'")?;
            let value = self.parse_value(depth)?;
            let separator = self.next_token()?;

            match separator.kind {
                JsonToken::Comma => {
                    // jsonc.org Appendix A: parsers MAY support trailing commas. When enabled, a
                    // comma directly before `}` closes the object and stays in the CST; a comma
                    // that precedes anything but a member name still errors below on the next
                    // iteration.
                    if self.allow_trailing_commas && self.peek_kind()? == JsonToken::RightBrace {
                        members.push(Member {
                            name,
                            colon,
                            value,
                            comma: Some(separator),
                        });
                        let close = self.next_token()?;

                        return Ok(Value::Object(ObjectNode {
                            open,
                            members,
                            close,
                        }));
                    }

                    members.push(Member {
                        name,
                        colon,
                        value,
                        comma: Some(separator),
                    });
                }
                JsonToken::RightBrace => {
                    members.push(Member {
                        name,
                        colon,
                        value,
                        comma: None,
                    });

                    return Ok(Value::Object(ObjectNode {
                        open,
                        members,
                        close: separator,
                    }));
                }
                JsonToken::Eof => {
                    return Err(JsoncError {
                        position: separator.start,
                        kind: JsoncErrorKind::UnexpectedEof {
                            expected: "',' or '}'",
                        },
                    });
                }
                found => {
                    return Err(JsoncError {
                        position: separator.start,
                        kind: JsoncErrorKind::Expected {
                            expected: "',' or '}'",
                            found,
                        },
                    });
                }
            }
        }
    }

    fn parse_array(&mut self, open: SyntaxToken, depth: usize) -> Result<Value, JsoncError> {
        let depth = self.enter_container(depth, &open)?;
        let mut elements = Vec::new();

        if self.peek_kind()? == JsonToken::RightBracket {
            let close = self.next_token()?;
            return Ok(Value::Array(ArrayNode {
                open,
                elements,
                close,
            }));
        }

        loop {
            let value = self.parse_value(depth)?;
            let separator = self.next_token()?;

            match separator.kind {
                JsonToken::Comma => {
                    // jsonc.org Appendix A: parsers MAY support trailing commas. See
                    // `parse_object`; `[,]` and `[1,,2]` still error through `parse_value`.
                    if self.allow_trailing_commas && self.peek_kind()? == JsonToken::RightBracket {
                        elements.push(Element {
                            value,
                            comma: Some(separator),
                        });
                        let close = self.next_token()?;

                        return Ok(Value::Array(ArrayNode {
                            open,
                            elements,
                            close,
                        }));
                    }

                    elements.push(Element {
                        value,
                        comma: Some(separator),
                    });
                }
                JsonToken::RightBracket => {
                    elements.push(Element { value, comma: None });

                    return Ok(Value::Array(ArrayNode {
                        open,
                        elements,
                        close: separator,
                    }));
                }
                JsonToken::Eof => {
                    return Err(JsoncError {
                        position: separator.start,
                        kind: JsoncErrorKind::UnexpectedEof {
                            expected: "',' or ']'",
                        },
                    });
                }
                found => {
                    return Err(JsoncError {
                        position: separator.start,
                        kind: JsoncErrorKind::Expected {
                            expected: "',' or ']'",
                            found,
                        },
                    });
                }
            }
        }
    }

    /// Consumes the next token, requiring `kind`.
    fn expect(
        &mut self,
        kind: JsonToken,
        describe: &'static str,
    ) -> Result<SyntaxToken, JsoncError> {
        let token = self.next_token()?;

        if token.kind == kind {
            Ok(token)
        } else if token.kind == JsonToken::Eof {
            Err(JsoncError {
                position: token.start,
                kind: JsoncErrorKind::UnexpectedEof { expected: describe },
            })
        } else {
            Err(JsoncError {
                position: token.start,
                kind: JsoncErrorKind::Expected {
                    expected: describe,
                    found: token.kind,
                },
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_fully_strict() {
        let parser = JsoncParser::new();
        assert!(!parser.allow_bom());
        assert!(!parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn builder_defaults_match_new() {
        let parser = JsoncParser::builder().build();
        assert!(!parser.allow_bom());
        assert!(!parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn builder_sets_options() {
        let parser = JsoncParser::builder()
            .allow_bom(true)
            .allow_trailing_commas(true)
            .max_depth(4)
            .build();
        assert!(parser.allow_bom());
        assert!(parser.allow_trailing_commas());
        assert_eq!(parser.max_depth(), 4);
    }

    #[test]
    fn default_depth_matches_strata_json() {
        assert_eq!(DEFAULT_MAX_DEPTH, strata_json::DEFAULT_MAX_DEPTH);
    }

    #[test]
    fn error_display_includes_position() {
        let Err(error) = JsoncParser::new().parse_str("[1 2]") else {
            panic!("expected a parse error");
        };
        let message = error.to_string();
        assert!(message.contains("expected ',' or ']'"), "{message}");
        assert!(message.contains("line 1"), "{message}");
        assert!(message.contains("byte 3"), "{message}");
    }

    #[test]
    fn bom_error_display_mentions_the_option() {
        let Err(error) = JsoncParser::new().parse_str("\u{FEFF}1") else {
            panic!("expected a parse error");
        };
        assert!(matches!(error.kind, JsoncErrorKind::UnexpectedBom));
        assert!(error.to_string().contains("allow_bom"), "{error}");
    }

    #[test]
    fn comment_positions_survive_into_errors() {
        // The error after a multiline comment must account for the comment's line breaks.
        let Err(error) = JsoncParser::new().parse_str("/* a\nb */ [1 2]") else {
            panic!("expected a parse error");
        };
        assert_eq!(error.position.line, 2);
    }
}
