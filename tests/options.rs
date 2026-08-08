//! Option passthrough to the parser and resolver-level policies.

use strata_jsonc::{
    JsoncError, JsoncErrorKind, JsoncNumber, JsoncResolver, JsoncValue, TriviaKind,
};

#[test]
fn strict_default_rejects_a_leading_bom() {
    let Err(error) = JsoncResolver::new().resolve_str("\u{FEFF}1") else {
        panic!("expected a BOM error");
    };

    let JsoncErrorKind::Parse(parse) = &error.kind else {
        panic!("expected a parse error, got {:?}", error.kind);
    };
    assert!(matches!(
        parse.kind,
        strata_jsonc_parser::JsoncErrorKind::UnexpectedBom
    ));
}

#[test]
fn allow_bom_retains_the_bom_as_trivia() -> Result<(), JsoncError> {
    let resolver = JsoncResolver::builder().allow_bom(true).build();
    let document = resolver.resolve_str("\u{FEFF}1")?;

    let [bom] = document.root().meta().leading() else {
        panic!("expected the BOM as leading trivia");
    };
    assert_eq!(bom.kind(), TriviaKind::Bom);
    assert_eq!(document.root().as_number(), Some(JsoncNumber::U64(1)));

    Ok(())
}

#[test]
fn strict_default_rejects_trailing_commas() {
    let Err(error) = JsoncResolver::new().resolve_str("[1,]") else {
        panic!("expected a trailing-comma error");
    };

    let JsoncErrorKind::Parse(parse) = &error.kind else {
        panic!("expected a parse error, got {:?}", error.kind);
    };
    assert!(matches!(
        parse.kind,
        strata_jsonc_parser::JsoncErrorKind::Expected { .. }
    ));
    assert!(error.to_string().contains("expected a JSON value"));
}

#[test]
fn allow_trailing_commas_tolerates_them_in_arrays_and_objects() -> Result<(), JsoncError> {
    let resolver = JsoncResolver::builder().allow_trailing_commas(true).build();

    let document = resolver.resolve_str("[1, 2,]")?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(array.len(), 2);

    let document = resolver.resolve_str("{\"a\": 1, /* last */}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };
    assert_eq!(
        object.get("a").and_then(JsoncValue::as_number),
        Some(JsoncNumber::U64(1))
    );

    // A comma with no preceding entry stays an error either way.
    assert!(resolver.resolve_str("[,]").is_err());

    Ok(())
}

#[test]
fn max_depth_passes_through() {
    let resolver = JsoncResolver::builder().max_depth(1).build();

    assert!(resolver.resolve_str("[1]").is_ok());

    let Err(error) = resolver.resolve_str("[[1]]") else {
        panic!("expected a depth error");
    };
    let JsoncErrorKind::Parse(parse) = &error.kind else {
        panic!("expected a parse error, got {:?}", error.kind);
    };
    assert!(matches!(
        parse.kind,
        strata_jsonc_parser::JsoncErrorKind::DepthLimitExceeded { limit: 1 }
    ));
}

#[test]
fn resolver_accessors_report_configuration() {
    let resolver = JsoncResolver::builder()
        .duplicate_keys(strata_jsonc::DuplicateKeys::Error)
        .lone_surrogates(strata_jsonc::LoneSurrogates::Replace)
        .allow_trailing_commas(true)
        .build();

    assert_eq!(
        resolver.duplicate_keys(),
        strata_jsonc::DuplicateKeys::Error
    );
    assert_eq!(
        resolver.lone_surrogates(),
        strata_jsonc::LoneSurrogates::Replace
    );
    assert!(resolver.parser().allow_trailing_commas());
    assert!(!resolver.parser().allow_bom());
}

#[test]
fn resolve_document_accepts_an_external_parse() -> Result<(), JsoncError> {
    let parser = strata_jsonc_parser::JsoncParser::builder()
        .allow_bom(true)
        .build();
    let Ok(parsed) = parser.parse_str("\u{FEFF}[true] // done") else {
        panic!("expected the parse to succeed");
    };

    let document = JsoncResolver::new().resolve_document(parsed)?;
    let Some(array) = document.root().as_array() else {
        panic!("expected an array");
    };
    assert_eq!(array.len(), 1);

    Ok(())
}
