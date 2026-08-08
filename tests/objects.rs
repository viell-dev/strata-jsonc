//! Object semantics: source order, duplicate members, and the duplicate-keys policy.

use strata_jsonc::{
    DuplicateKeys, JsoncError, JsoncErrorKind, JsoncMember, JsoncNumber, JsoncResolver, JsoncValue,
};

fn object(input: &str) -> Result<strata_jsonc::JsoncDocument, JsoncError> {
    JsoncResolver::new().resolve_str(input)
}

#[test]
fn members_keep_source_order() -> Result<(), JsoncError> {
    let document = object(r#"{"z": 1, "a": 2, "m": 3}"#)?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let names: Vec<&str> = object.members().iter().map(JsoncMember::name).collect();
    assert_eq!(names, vec!["z", "a", "m"]);
    assert_eq!(object.len(), 3);
    assert!(!object.is_empty());
    assert!(object.contains("a"));
    assert!(!object.contains("q"));

    Ok(())
}

#[test]
fn member_names_are_decoded() -> Result<(), JsoncError> {
    let document = object(r#"{"a": 1}"#)?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    assert_eq!(
        object.get("a").and_then(JsoncValue::as_number),
        Some(JsoncNumber::U64(1))
    );

    Ok(())
}

#[test]
fn get_resolves_to_the_last_occurrence() -> Result<(), JsoncError> {
    let document = object(r#"{"a": 1, "b": 2, "a": 3}"#)?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    assert_eq!(
        object.get("a").and_then(JsoncValue::as_number),
        Some(JsoncNumber::U64(3))
    );
    assert_eq!(object.len(), 3);

    let all: Vec<Option<JsoncNumber>> = object
        .get_all("a")
        .map(|member| member.value().as_number())
        .collect();
    assert_eq!(
        all,
        vec![Some(JsoncNumber::U64(1)), Some(JsoncNumber::U64(3))]
    );

    Ok(())
}

#[test]
fn duplicates_reports_shadowed_members() -> Result<(), JsoncError> {
    let document = object(r#"{"a": 1, "b": 2, "a": 3, "b": 4, "c": 5}"#)?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let shadowed: Vec<(&str, Option<JsoncNumber>)> = object
        .duplicates()
        .map(|member| (member.name(), member.value().as_number()))
        .collect();
    assert_eq!(
        shadowed,
        vec![
            ("a", Some(JsoncNumber::U64(1))),
            ("b", Some(JsoncNumber::U64(2))),
        ]
    );

    Ok(())
}

#[test]
fn duplicate_keys_error_policy_rejects_at_the_repeated_name() {
    let resolver = JsoncResolver::builder()
        .duplicate_keys(DuplicateKeys::Error)
        .build();
    let Err(error) = resolver.resolve_str(r#"{"a": 1, "a": 2}"#) else {
        panic!("expected a duplicate-member error");
    };

    let JsoncErrorKind::DuplicateMember { name } = &error.kind else {
        panic!("expected DuplicateMember, got {:?}", error.kind);
    };
    assert_eq!(name, "a");
    // The repeated (second) name token starts at byte 9.
    assert_eq!(error.position.byte(), 9);
}

#[test]
fn duplicate_detection_uses_decoded_names() {
    let resolver = JsoncResolver::builder()
        .duplicate_keys(DuplicateKeys::Error)
        .build();
    let Err(error) = resolver.resolve_str(r#"{"a": 1, "\u0061": 2}"#) else {
        panic!("expected a duplicate-member error for the escaped spelling");
    };

    assert!(matches!(
        error.kind,
        JsoncErrorKind::DuplicateMember { ref name } if name == "a"
    ));
}

#[test]
fn commented_members_resolve_like_plain_ones() -> Result<(), JsoncError> {
    let document = object("{\n  // first\n  \"a\": 1,\n  /* second */ \"b\": 2\n}")?;
    let Some(object) = document.root().as_object() else {
        panic!("expected an object");
    };

    let names: Vec<&str> = object.members().iter().map(JsoncMember::name).collect();
    assert_eq!(names, vec!["a", "b"]);
    assert_eq!(
        object.get("b").and_then(JsoncValue::as_number),
        Some(JsoncNumber::U64(2))
    );

    Ok(())
}

#[test]
fn nested_objects_resolve() -> Result<(), JsoncError> {
    let document = object(r#"{"outer": {"inner": [1]}}"#)?;
    let Some(outer) = document.root().as_object() else {
        panic!("expected an object");
    };
    let Some(inner) = outer.get("outer").and_then(JsoncValue::as_object) else {
        panic!("expected a nested object");
    };
    let Some(array) = inner.get("inner").and_then(JsoncValue::as_array) else {
        panic!("expected a nested array");
    };

    assert_eq!(array.len(), 1);

    Ok(())
}
