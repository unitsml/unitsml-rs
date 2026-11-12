//! Caller registrations apply only to the directly named unit (unitsml-ruby
//! Conversion::Register scope); chained units keep their UnitsDB definition.
use unitsml_rs::{rational, Register, Registration, UnitTerm};

fn term(unit: &str) -> UnitTerm {
    UnitTerm {
        unit: unit.to_owned(),
        prefix: None,
        power: rational("1").unwrap(),
    }
}

fn mass_register(unit: &str, factor: &str) -> Register {
    let mut register = Register::default();
    let dimensions = [("Mass".to_owned(), rational("1").unwrap())]
        .into_iter()
        .collect();
    register
        .insert(
            unit,
            Registration {
                dimensions,
                factor: rational(factor).unwrap(),
                offset: rational("0").unwrap(),
            },
        )
        .unwrap();
    register
}

#[test]
fn a_directly_named_registered_unit_uses_the_registration() {
    let register = mass_register("NISTu201", "0.45359237");
    let lb = register.resolve(&[term("NISTu201")]).unwrap();
    assert_eq!(lb.factor(), &rational("0.45359237").unwrap());
}

#[test]
fn a_registration_does_not_reach_units_defined_through_root_units() {
    // cwt (US) is hecto-pound in UnitsDB; registering the pound does not
    // define the hundredweight, exactly as in unitsml-ruby.
    let register = mass_register("NISTu201", "0.45359237");
    assert!(register.resolve(&[term("NISTu230")]).is_err());
}
