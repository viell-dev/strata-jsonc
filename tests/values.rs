//! Owned-value resolution: scalars, string decoding, and number conversion.

use strata_jsonc::{
    JsoncError, JsoncErrorKind, JsoncNumber, JsoncResolver, JsoncValue, LoneSurrogates,
    NumberFormat,
};

fn root(input: &str) -> Result<JsoncValue, JsoncError> {
    Ok(JsoncResolver::new().resolve_str(input)?.into_parts().0)
}

#[test]
fn scalars_resolve_to_owned_values() -> Result<(), JsoncError> {
    assert!(root("null")?.is_null());
    assert_eq!(root("true")?.as_bool(), Some(true));
    assert_eq!(root("false")?.as_bool(), Some(false));
    assert_eq!(root("42")?.as_number(), Some(JsoncNumber::U64(42)));
    assert_eq!(root(r#""hello""#)?.as_str(), Some("hello"));

    Ok(())
}

#[test]
fn scalars_resolve_with_comments_around_them() -> Result<(), JsoncError> {
    assert_eq!(
        root("// leading\n42 /* trailing */")?.as_number(),
        Some(JsoncNumber::U64(42))
    );

    Ok(())
}

#[test]
fn strings_decode_every_simple_escape() -> Result<(), JsoncError> {
    let value = root(r#""\"\\\/\b\f\n\r\t""#)?;

    assert_eq!(value.as_str(), Some("\"\\/\u{08}\u{0C}\n\r\t"));

    Ok(())
}

#[test]
fn strings_decode_unicode_escapes_and_surrogate_pairs() -> Result<(), JsoncError> {
    assert_eq!(
        root(r#""\u0041\u00e9\u20AC""#)?.as_str(),
        Some("A\u{E9}\u{20AC}")
    );
    assert_eq!(root(r#""\uD83D\uDE00""#)?.as_str(), Some("\u{1F600}"));
    assert_eq!(root(r#""\u0000""#)?.as_str(), Some("\u{0}"));

    Ok(())
}

#[test]
fn comment_delimiters_inside_strings_are_content() -> Result<(), JsoncError> {
    assert_eq!(
        root(r#""not // a comment""#)?.as_str(),
        Some("not // a comment")
    );
    assert_eq!(
        root(r#""nor /* this */""#)?.as_str(),
        Some("nor /* this */")
    );

    Ok(())
}

#[test]
fn multibyte_content_survives_verbatim() -> Result<(), JsoncError> {
    assert_eq!(
        root("\"caf\u{E9} \u{1F600}\"")?.as_str(),
        Some("caf\u{E9} \u{1F600}")
    );

    Ok(())
}

#[test]
fn lone_surrogates_error_by_default() {
    let Err(error) = root(r#""\uD800""#) else {
        panic!("expected a lone-surrogate error");
    };

    assert!(matches!(
        error.kind,
        JsoncErrorKind::LoneSurrogate { unit: 0xD800 }
    ));
    // Byte 0 is the quote, byte 1 the backslash of the lone escape.
    assert_eq!(error.position.byte(), 1);
}

#[test]
fn lone_surrogates_replace_under_policy() -> Result<(), JsoncError> {
    let resolver = JsoncResolver::builder()
        .lone_surrogates(LoneSurrogates::Replace)
        .build();
    let document = resolver.resolve_str(r#""a\uDC00b""#)?;

    assert_eq!(document.root().as_str(), Some("a\u{FFFD}b"));

    Ok(())
}

#[test]
fn numbers_preserve_integer_identity() -> Result<(), JsoncError> {
    assert_eq!(
        root("18446744073709551615")?.as_number(),
        Some(JsoncNumber::U64(u64::MAX))
    );
    assert_eq!(
        root("-9223372036854775808")?.as_number(),
        Some(JsoncNumber::I64(i64::MIN))
    );
    assert_eq!(root("0")?.as_number(), Some(JsoncNumber::U64(0)));

    Ok(())
}

#[test]
fn fractions_exponents_and_overflow_convert_to_f64() -> Result<(), JsoncError> {
    assert_eq!(root("2.5e-3")?.as_f64(), Some(2.5e-3));
    assert_eq!(root("-0.5")?.as_f64(), Some(-0.5));
    assert_eq!(
        root("18446744073709551616")?.as_f64(),
        Some(18_446_744_073_709_551_616.0)
    );

    Ok(())
}

#[test]
fn negative_zero_keeps_its_sign() -> Result<(), JsoncError> {
    let Some(JsoncNumber::F64(value)) = root("-0")?.as_number() else {
        panic!("expected -0 to resolve as a float");
    };

    assert_eq!(value.to_bits(), (-0.0f64).to_bits());

    Ok(())
}

#[test]
fn arrays_resolve_recursively() -> Result<(), JsoncError> {
    let value = root(r#"[1, "two", [true, null]]"#)?;
    let Some(array) = value.as_array() else {
        panic!("expected an array");
    };

    assert_eq!(array.len(), 3);
    assert_eq!(
        array.get(0).and_then(JsoncValue::as_number),
        Some(JsoncNumber::U64(1))
    );
    assert_eq!(array.get(1).and_then(JsoncValue::as_str), Some("two"));

    let Some(inner) = array.get(2).and_then(JsoncValue::as_array) else {
        panic!("expected a nested array");
    };
    assert_eq!(inner.get(0).and_then(JsoncValue::as_bool), Some(true));
    assert!(inner.get(1).is_some_and(JsoncValue::is_null));
    assert!(!array.is_empty());

    Ok(())
}

#[test]
fn parse_errors_pass_through() {
    let Err(error) = root("[1 2]") else {
        panic!("expected a parse error");
    };

    assert!(matches!(error.kind, JsoncErrorKind::Parse(_)));
    assert!(error.to_string().contains("expected ',' or ']'"));
}

#[test]
fn number_formats_surface_the_written_form() -> Result<(), JsoncError> {
    assert_eq!(root("42")?.number_format(), Some(NumberFormat::Integer));
    assert_eq!(root("-0.5")?.number_format(), Some(NumberFormat::Decimal));
    assert_eq!(
        root("1.23e+5")?.number_format(),
        Some(NumberFormat::Exponential)
    );
    assert_eq!(root("true")?.number_format(), None);

    Ok(())
}
