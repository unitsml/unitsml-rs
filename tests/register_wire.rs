use serde_json::{json, Value};
use unitsml_rs::{
    decode_register, evaluate_json, rational, Error, Quantity, Register, UnitTerm, WireError,
};

fn canonical(register: &Register, value: &str, unit: &str) -> String {
    Quantity::new(
        rational(value).unwrap(),
        register.resolve(&[UnitTerm::new(unit)]).unwrap(),
    )
    .canonical_magnitude()
    .to_string()
}

#[test]
fn original_wire_bytes_and_error_precedence_are_preserved() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/register-wire-v1.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    for case in cases {
        let request = case["request_json"].as_str().unwrap().as_bytes();
        assert_eq!(
            evaluate_json(request).to_string(),
            case["expected_wire"].as_str().unwrap(),
            "{}",
            case["id"]
        );
    }
}

#[test]
fn public_register_helper_keeps_exact_factor_offset_and_dimensions(
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = json!([
        {"unit":"NISTu230","dimensions":{"Mass":"1"},"factor":"1/3"},
        {"unit":"NISTu23","dimensions":{"ThermodynamicTemperature":"1"},"factor":"1","offset":"273.15"},
        {"unit":"W","dimensions":{"Mass":"1","Length":"2","Time":"-3"},"factor":"1"}
    ]);
    let register = decode_register(Some(&entries))?;
    assert_eq!(canonical(&register, "2", "NISTu230"), "2/3");
    assert_eq!(canonical(&register, "2", "degC"), "5503/20");
    let power = register.resolve(&[UnitTerm::new("W")])?;
    assert_eq!(power.dimensions()["Length"], rational("2")?);
    assert_eq!(power.dimensions()["Mass"], rational("1")?);
    assert_eq!(power.dimensions()["Time"], rational("-3")?);
    Ok(())
}

#[test]
fn caller_registers_are_independent_and_failure_does_not_escape_a_partial_register() {
    let first = json!([{"unit":"NISTu230","dimensions":{"Mass":"1"},"factor":"50"}]);
    let second = json!([{"unit":"NISTu230","dimensions":{"Mass":"1"},"factor":"60"}]);
    let a = decode_register(Some(&first)).unwrap();
    let b = decode_register(Some(&second)).unwrap();
    assert_eq!(canonical(&a, "2", "NISTu230"), "100");
    assert_eq!(canonical(&b, "2", "NISTu230"), "120");
    assert_eq!(canonical(&a, "2", "NISTu230"), "100");
    let broken = json!([
        {"unit":"NISTu230","dimensions":{"Mass":"1"},"factor":"50"},
        {"unit":"NISTu230","dimensions":{"Mass":"1"},"factor":"-1"}
    ]);
    assert!(matches!(
        decode_register(Some(&broken)),
        Err(WireError::Algebra(Error::InvalidRegister(_)))
    ));
    assert!(matches!(
        decode_register(None)
            .unwrap()
            .resolve(&[UnitTerm::new("NISTu230")]),
        Err(Error::MissingConversion(_))
    ));
}

#[test]
fn public_helper_retains_typed_malformed_and_resource_errors() {
    assert!(
        matches!(decode_register(Some(&Value::Null)), Err(WireError::Invalid(detail)) if detail == "invalid or missing register")
    );
    let invalid = json!([{"unit":"m","dimensions":{"Mass":"1"},"factor":"1"}]);
    assert!(matches!(
        decode_register(Some(&invalid)),
        Err(WireError::Algebra(Error::InvalidRegister(_)))
    ));
    let too_many = Value::Array(vec![
        json!({"unit":"m","dimensions":{"Length":"1"},"factor":"1"});
        257
    ]);
    assert!(matches!(
        decode_register(Some(&too_many)),
        Err(WireError::Algebra(Error::ResourceLimit))
    ));
}
