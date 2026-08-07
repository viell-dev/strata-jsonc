//! JSONC token rules: the JSON rules lifted into a wider kind, plus the two comment shapes.
//!
//! JSONC (jsonc.org) extends RFC 8259 only at the lexical level: `//` line comments and
//! non-nesting `/* */` block comments may appear anywhere insignificant whitespace is allowed.
//! This module therefore reuses `strata-json`'s rule structs unchanged — lifted into
//! [`JsoncToken`] via [`MatcherExt::map_kind`](strata_lexer::MatcherExt::map_kind) — and adds
//! [`LineCommentRule`] and [`BlockCommentRule`]. Reused rules keep their validation: `01` is
//! still a lexical error, strings still reject bad escapes, and so on.
//!
//! Both comment rules are exported individually so a further superset can compose on JSONC the
//! same way this crate composes on JSON: lift the rules it keeps, add what differs.

use strata_json::{
    BomRule, JsonToken, LiteralRule, NumberRule, PunctRule, StringRule, WhitespaceRule,
};
use strata_lexer::{
    Candidate, LexError, Lookahead, MapMatcher, Match, Matcher, MatcherExt, MatcherSet, Position,
    SpecScanner,
};

/// Token kinds produced by the JSONC token rules.
///
/// Every significant JSON shape is carried unchanged inside [`JsoncToken::Json`]; the two comment
/// kinds are the only additions, and both are trivia — JSONC adds no significant token kinds
/// (jsonc.org: "Removing all comments MUST yield the same data representation of the JSON
/// values").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JsoncToken {
    /// A token shared with RFC 8259 JSON, produced by a lifted `strata-json` rule.
    Json(JsonToken),
    /// A `//` line comment, ending before the line break (or at end of input).
    LineComment,
    /// A `/* */` block comment, span covering both delimiters. Block comments do not nest.
    BlockComment,
    /// End of input.
    Eof,
}

impl JsoncToken {
    /// Returns whether this kind is trivia — grammatically insignificant source content that the
    /// CST retains instead of discarding. Both comment kinds are trivia, alongside JSON's
    /// whitespace and byte-order mark.
    pub const fn is_trivia(self) -> bool {
        match self {
            Self::Json(token) => token.is_trivia(),
            Self::LineComment | Self::BlockComment => true,
            Self::Eof => false,
        }
    }

    /// A short human-readable description for diagnostics.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Json(token) => token.describe(),
            Self::LineComment => "a line comment",
            Self::BlockComment => "a block comment",
            Self::Eof => "end of input",
        }
    }
}

/// The kind-lifting function shared by all reused JSON rules.
type Wrap = fn(JsonToken) -> JsoncToken;

/// The six strict RFC 8259 rules, each lifted into [`JsoncToken`].
type LiftedJsonRules = (
    MapMatcher<WhitespaceRule, Wrap>,
    MapMatcher<PunctRule, Wrap>,
    MapMatcher<LiteralRule, Wrap>,
    MapMatcher<StringRule, Wrap>,
    MapMatcher<NumberRule, Wrap>,
    MapMatcher<BomRule, Wrap>,
);

/// The strict jsonc.org rule set: strict RFC 8259 rules plus the two comment rules.
///
/// Like `strata-json`'s set, this is fully monomorphic and cheap to clone. `BomRule` is included
/// for the same reason as upstream: the rules describe which shapes *exist*, while the parser
/// owns the policy (reject strictly, or retain as trivia under `allow_bom`). Trailing commas
/// need no extra rule — `,` is already a JSON token; the parser's `allow_trailing_commas` option
/// is purely grammatical.
#[derive(Debug, Clone, Copy)]
pub struct StrictRuleSet {
    comments: (LineCommentRule, BlockCommentRule),
    json: LiftedJsonRules,
}

impl Default for StrictRuleSet {
    fn default() -> Self {
        strict_rule_set()
    }
}

impl MatcherSet for StrictRuleSet {
    type Kind = JsoncToken;

    fn measure(&self, look: &mut Lookahead<'_>) -> Result<Vec<Candidate<JsoncToken>>, LexError> {
        let mut candidates = self.comments.measure(look)?;
        candidates.extend(self.json.measure(look)?);
        Ok(candidates)
    }
}

/// Returns the strict jsonc.org rule set.
pub fn strict_rule_set() -> StrictRuleSet {
    let wrap: Wrap = JsoncToken::Json;

    StrictRuleSet {
        comments: (LineCommentRule, BlockCommentRule),
        json: (
            WhitespaceRule.map_kind(wrap),
            PunctRule.map_kind(wrap),
            LiteralRule.map_kind(wrap),
            StringRule.map_kind(wrap),
            NumberRule.map_kind(wrap),
            BomRule.map_kind(wrap),
        ),
    }
}

/// Returns a [`SpecScanner`] over the strict rule set with [`JsoncToken::Eof`] as its
/// end-of-input token.
pub fn strict_scanner() -> SpecScanner<StrictRuleSet, JsoncToken> {
    SpecScanner::new(strict_rule_set(), JsoncToken::Eof)
}

/// Advances `start` over `bytes`, mirroring the lexer's position rules (CR, LF, and CRLF each
/// count as one line break).
fn position_after(start: Position, bytes: &[u8]) -> Position {
    let mut position = start;
    let mut previous_was_cr = false;

    for &byte in bytes {
        position.byte = position.byte.saturating_add(1);

        match byte {
            b'\r' => {
                position.line = position.line.saturating_add(1);
                position.column = 1;
                previous_was_cr = true;
            }
            b'\n' if previous_was_cr => previous_was_cr = false,
            b'\n' => {
                position.line = position.line.saturating_add(1);
                position.column = 1;
            }
            _ => {
                position.column = position.column.saturating_add(1);
                previous_was_cr = false;
            }
        }
    }

    position
}

/// Validates that the measured comment is valid UTF-8 and produces the match.
///
/// The jsonc.org ABNF defines comment content in characters (code points), and RFC 8259 §8.1
/// requires the whole text to be UTF-8, so invalid UTF-8 inside a comment is a lexical error at
/// the offending byte — the same policy `strata-json` applies inside strings.
fn finish_comment(
    look: &mut Lookahead<'_>,
    start: Position,
    len: usize,
    kind: JsoncToken,
) -> Result<Option<Match<JsoncToken>>, LexError> {
    let raw = look.peek(len)?;

    match core::str::from_utf8(raw) {
        Ok(_) => Ok(Some(Match::new(kind, len))),
        Err(error) => {
            let valid = raw.get(..error.valid_up_to()).unwrap_or(raw);
            Err(LexError::unrecognized(position_after(start, valid)))
        }
    }
}

/// Matches a `//` line comment (jsonc.org §"Comments").
///
/// "Single-line comments start with `//` and continue until a line ending is encountered." The
/// measured span excludes the line ending: the following CR/LF is ordinary insignificant
/// whitespace and lexes as a whitespace token, which keeps blank-line counting uniform. A comment
/// terminated by end of input is valid (the ABNF makes the line ending optional).
///
/// `#` comments are *not* JSONC ("JSONC must remain a subset of JavaScript syntax"); no rule
/// matches `#`, so it surfaces as an unrecognized byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCommentRule;

impl Matcher for LineCommentRule {
    type Kind = JsoncToken;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<JsoncToken>>, LexError> {
        if look.peek(2)? != b"//" {
            return Ok(None);
        }

        let start = look.position();
        let mut index: usize = 2;

        loop {
            let bytes = look.peek(index.saturating_add(1))?;

            match bytes.get(index) {
                Some(b'\n' | b'\r') | None => {
                    return finish_comment(look, start, index, JsoncToken::LineComment);
                }
                Some(_) => index = index.saturating_add(1),
            }
        }
    }
}

/// Matches a `/* */` block comment (jsonc.org §"Comments").
///
/// "Multi-line comments cannot be nested. The closing of the nested comment will be interpreted
/// as the end of the outer comment." — the rule therefore ends the comment at the *first* `*/`
/// after the opener. The measured span covers both delimiters. An unclosed block comment is
/// [`LexErrorKind::UnterminatedDelimiter`](strata_lexer::LexErrorKind::UnterminatedDelimiter)
/// reported at the opening `/*`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockCommentRule;

impl Matcher for BlockCommentRule {
    type Kind = JsoncToken;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<JsoncToken>>, LexError> {
        if look.peek(2)? != b"/*" {
            return Ok(None);
        }

        let start = look.position();
        // Content starts after the opener; `/*/` is unterminated, not a comment whose closer
        // shares the opener's `*`.
        let mut index: usize = 2;

        loop {
            let bytes = look.peek(index.saturating_add(2))?;

            match bytes.get(index) {
                None => return Err(LexError::unterminated_delimiter(start)),
                Some(b'*') if bytes.get(index.saturating_add(1)) == Some(&b'/') => {
                    let len = index.saturating_add(2);
                    return finish_comment(look, start, len, JsoncToken::BlockComment);
                }
                Some(_) => index = index.saturating_add(1),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use strata_lexer::{LexErrorKind, Lexer};

    fn lex_bytes(input: &[u8]) -> Result<Vec<(JsoncToken, u64, u64)>, LexError> {
        let mut lexer = Lexer::new(Cursor::new(input.to_vec()), strict_scanner());
        let mut out = Vec::new();

        loop {
            let token = lexer.next_token()?;
            let done = token.kind == JsoncToken::Eof;
            out.push((token.kind, token.span.start, token.span.end));

            if done {
                break;
            }
        }

        Ok(out)
    }

    fn lex(input: &str) -> Result<Vec<(JsoncToken, u64, u64)>, LexError> {
        lex_bytes(input.as_bytes())
    }

    fn lex_error(input: &str) -> LexError {
        match lex(input) {
            Err(error) => error,
            Ok(tokens) => panic!("expected a lexical error, got {tokens:?}"),
        }
    }

    #[test]
    fn line_comment_ends_before_the_line_break() -> Result<(), LexError> {
        assert_eq!(
            lex("// note\n1")?,
            vec![
                (JsoncToken::LineComment, 0, 7),
                (JsoncToken::Json(JsonToken::Whitespace), 7, 8),
                (JsoncToken::Json(JsonToken::Number), 8, 9),
                (JsoncToken::Eof, 9, 9),
            ]
        );

        Ok(())
    }

    #[test]
    fn line_comment_may_end_at_eof() -> Result<(), LexError> {
        assert_eq!(
            lex("1 // note")?,
            vec![
                (JsoncToken::Json(JsonToken::Number), 0, 1),
                (JsoncToken::Json(JsonToken::Whitespace), 1, 2),
                (JsoncToken::LineComment, 2, 9),
                (JsoncToken::Eof, 9, 9),
            ]
        );

        Ok(())
    }

    #[test]
    fn line_comment_ends_before_a_carriage_return() -> Result<(), LexError> {
        let tokens = lex("// note\r\n1")?;
        assert_eq!(tokens.first(), Some(&(JsoncToken::LineComment, 0, 7)));

        Ok(())
    }

    #[test]
    fn empty_line_comment_matches() -> Result<(), LexError> {
        assert_eq!(
            lex("//")?,
            vec![(JsoncToken::LineComment, 0, 2), (JsoncToken::Eof, 2, 2)]
        );

        Ok(())
    }

    #[test]
    fn block_comment_spans_both_delimiters() -> Result<(), LexError> {
        assert_eq!(
            lex("/* note */1")?,
            vec![
                (JsoncToken::BlockComment, 0, 10),
                (JsoncToken::Json(JsonToken::Number), 10, 11),
                (JsoncToken::Eof, 11, 11),
            ]
        );

        Ok(())
    }

    #[test]
    fn empty_block_comment_matches() -> Result<(), LexError> {
        assert_eq!(
            lex("/**/")?,
            vec![(JsoncToken::BlockComment, 0, 4), (JsoncToken::Eof, 4, 4)]
        );

        Ok(())
    }

    #[test]
    fn star_runs_do_not_confuse_the_closer() -> Result<(), LexError> {
        assert_eq!(
            lex("/***/")?,
            vec![(JsoncToken::BlockComment, 0, 5), (JsoncToken::Eof, 5, 5)]
        );

        Ok(())
    }

    #[test]
    fn block_comment_may_span_lines() -> Result<(), LexError> {
        let tokens = lex("/* a\nb */ 1")?;
        assert_eq!(tokens.first(), Some(&(JsoncToken::BlockComment, 0, 9)));

        Ok(())
    }

    #[test]
    fn block_comment_ends_at_the_first_closer() -> Result<(), LexError> {
        // Non-nesting: the inner `/*` is plain content and the comment ends at the first `*/`.
        assert_eq!(
            lex("/* /* */ 1")?,
            vec![
                (JsoncToken::BlockComment, 0, 8),
                (JsoncToken::Json(JsonToken::Whitespace), 8, 9),
                (JsoncToken::Json(JsonToken::Number), 9, 10),
                (JsoncToken::Eof, 10, 10),
            ]
        );

        Ok(())
    }

    #[test]
    fn unterminated_block_comment_is_reported_at_the_opener() {
        let error = lex_error("1 /* open");
        assert!(matches!(error.kind, LexErrorKind::UnterminatedDelimiter));
        assert_eq!(error.position.byte, 2);
    }

    #[test]
    fn slash_star_alone_is_unterminated() {
        // `/*/` cannot close with the opener's own `*`.
        let error = lex_error("/*/");
        assert!(matches!(error.kind, LexErrorKind::UnterminatedDelimiter));
        assert_eq!(error.position.byte, 0);
    }

    #[test]
    fn hash_is_not_a_comment() {
        let error = lex_error("# note");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte, 0);
    }

    #[test]
    fn lone_slash_is_unrecognized() {
        let error = lex_error("/ 1");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte, 0);
    }

    #[test]
    fn comments_accept_multibyte_utf8() -> Result<(), LexError> {
        let line = "// caf\u{E9} \u{1F600}";
        let tokens = lex(line)?;
        let expected_end = u64::try_from(line.len()).unwrap_or(u64::MAX);
        assert_eq!(
            tokens.first(),
            Some(&(JsoncToken::LineComment, 0, expected_end))
        );

        let block = "/* caf\u{E9} */";
        let tokens = lex(block)?;
        let expected_end = u64::try_from(block.len()).unwrap_or(u64::MAX);
        assert_eq!(
            tokens.first(),
            Some(&(JsoncToken::BlockComment, 0, expected_end))
        );

        Ok(())
    }

    #[test]
    fn invalid_utf8_in_comments_is_rejected_at_the_byte() {
        // Bytes: 0 `/`, 1 `/`, 2 ` `, 3 0xFF.
        let Err(error) = lex_bytes(b"// \xFF") else {
            panic!("expected a lexical error");
        };
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte, 3);

        // Bytes: 0 `/`, 1 `*`, 2 `\n`, 3 0xFF ... — the offending byte sits on line 2.
        let Err(error) = lex_bytes(b"/*\n\xFF*/") else {
            panic!("expected a lexical error");
        };
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte, 3);
        assert_eq!(error.position.line, 2);
        assert_eq!(error.position.column, 1);
    }

    #[test]
    fn control_characters_are_valid_comment_content() -> Result<(), LexError> {
        // The ABNF allows raw control characters (other than line endings) in comments.
        let tokens = lex("// a\tb\u{0}c")?;
        assert_eq!(
            tokens.first().map(|token| token.0),
            Some(JsoncToken::LineComment)
        );

        Ok(())
    }

    #[test]
    fn reused_json_rules_keep_their_validation() {
        let error = lex_error("01");
        assert!(matches!(error.kind, LexErrorKind::Unrecognized));
        assert_eq!(error.position.byte, 1);
    }

    #[test]
    fn slashes_inside_strings_are_content() -> Result<(), LexError> {
        let tokens = lex(r#""//not a comment""#)?;
        assert_eq!(
            tokens.first().map(|token| token.0),
            Some(JsoncToken::Json(JsonToken::String))
        );

        Ok(())
    }

    #[test]
    fn trivia_classification_covers_comments() {
        assert!(JsoncToken::LineComment.is_trivia());
        assert!(JsoncToken::BlockComment.is_trivia());
        assert!(JsoncToken::Json(JsonToken::Whitespace).is_trivia());
        assert!(JsoncToken::Json(JsonToken::Bom).is_trivia());
        assert!(!JsoncToken::Json(JsonToken::Number).is_trivia());
        assert!(!JsoncToken::Eof.is_trivia());
    }

    #[test]
    fn position_after_walks_crlf_as_one_break() {
        let start = Position {
            byte: 0,
            line: 1,
            column: 1,
        };
        let end = position_after(start, b"a\r\nb");
        assert_eq!(end.byte, 4);
        assert_eq!(end.line, 2);
        assert_eq!(end.column, 2);
    }
}
