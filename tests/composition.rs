//! Superset composition on JSONC: a miniature "JSONC plus `#` comments" grammar built from this
//! crate's public token rules, proving the composition chain continues — exactly as this crate
//! composes on `strata-json`, a further superset can compose on `strata-jsonc`.

use std::io::Cursor;
use strata_json::{JsonToken, NumberRule, PunctRule, WhitespaceRule};
use strata_jsonc::{BlockCommentRule, JsoncToken, LineCommentRule};
use strata_lexer::{
    LexError, Lexer, Lookahead, Match, Matcher, MatcherExt, MatcherVec, SpecScanner,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuperToken {
    Jsonc(JsoncToken),
    HashComment,
    Eof,
}

fn lift_jsonc(kind: JsoncToken) -> SuperToken {
    SuperToken::Jsonc(kind)
}

fn lift_json(kind: JsonToken) -> SuperToken {
    SuperToken::Jsonc(JsoncToken::Json(kind))
}

/// A `#` line comment rule — the one shape this superset adds over JSONC.
#[derive(Debug, Clone, Copy, Default)]
struct HashCommentRule;

impl Matcher for HashCommentRule {
    type Kind = SuperToken;

    fn try_match(&self, look: &mut Lookahead<'_>) -> Result<Option<Match<SuperToken>>, LexError> {
        if look.peek_byte()? != Some(b'#') {
            return Ok(None);
        }

        let mut len: usize = 1;

        loop {
            let bytes = look.peek(len.saturating_add(1))?;

            match bytes.get(len) {
                Some(b'\n' | b'\r') | None => {
                    return Ok(Match::from_len(SuperToken::HashComment, len));
                }
                Some(_) => len = len.saturating_add(1),
            }
        }
    }
}

fn super_rules() -> MatcherVec<SuperToken> {
    vec![
        HashCommentRule.boxed(),
        // JSONC's own additions, lifted from this crate.
        LineCommentRule.map_kind(lift_jsonc).boxed(),
        BlockCommentRule.map_kind(lift_jsonc).boxed(),
        // A subset of the JSON rules, lifted through both layers.
        WhitespaceRule.map_kind(lift_json).boxed(),
        PunctRule.map_kind(lift_json).boxed(),
        NumberRule.map_kind(lift_json).boxed(),
    ]
}

fn lex_kinds(input: &str) -> Result<Vec<SuperToken>, LexError> {
    let scanner = SpecScanner::new(super_rules(), SuperToken::Eof);
    let mut lexer = Lexer::new(Cursor::new(input.as_bytes().to_vec()), scanner);
    let mut kinds = Vec::new();

    loop {
        let token = lexer.next_token()?;
        let done = token.kind == SuperToken::Eof;
        kinds.push(token.kind);

        if done {
            return Ok(kinds);
        }
    }
}

#[test]
fn jsonc_rules_lex_under_the_wider_kind() -> Result<(), LexError> {
    let kinds = lex_kinds("[1] // note")?;

    assert_eq!(
        kinds,
        vec![
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::LeftBracket)),
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::Number)),
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::RightBracket)),
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::Whitespace)),
            SuperToken::Jsonc(JsoncToken::LineComment),
            SuperToken::Eof,
        ]
    );

    Ok(())
}

#[test]
fn added_hash_rule_extends_the_grammar() -> Result<(), LexError> {
    let kinds = lex_kinds("1 # now allowed")?;

    assert_eq!(
        kinds,
        vec![
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::Number)),
            SuperToken::Jsonc(JsoncToken::Json(JsonToken::Whitespace)),
            SuperToken::HashComment,
            SuperToken::Eof,
        ]
    );

    Ok(())
}

#[test]
fn lifted_jsonc_rules_keep_their_validation() {
    // The block-comment rule still reports an unterminated comment through two lift layers.
    let Err(error) = lex_kinds("/* open") else {
        panic!("expected the lifted block-comment rule to keep its termination check");
    };
    assert_eq!(error.position.byte(), 0);
}
