use std::cmp::Ordering;
use unitsml_rs::{rational, Error, Quantity, Rational, Register, Registration, Scale, UnitTerm};

fn n(value: &str) -> Rational {
    rational(value).unwrap()
}

fn q(db: &Register, value: &str, unit: &str) -> Quantity {
    Quantity::new(n(value), db.resolve(&[UnitTerm::new(unit)]).unwrap())
}

#[test]
fn affine_endpoints_and_scale() {
    let db = Register::default();
    let c = q(&db, "20", "degC");
    let k = q(&db, "293.15", "K");
    assert_eq!(c.compare(&k), Ok(Ordering::Equal));
    assert_eq!(c.convert(k.unit()).unwrap(), k);
    assert_eq!(c.scale(), Scale::Interval);
    assert!(matches!(
        c.scale_by(&n("2")),
        Err(Error::ScaleOperation { .. })
    ));
    let difference = c.subtract(&q(&db, "15", "degC")).unwrap();
    assert_eq!(difference.scale(), Scale::Difference);
    let difference = difference.convert(k.unit()).unwrap();
    assert_eq!(difference.magnitude(), &n("5"));
    assert_eq!(difference.scale(), Scale::Difference);
    assert_eq!(difference.add(&c).unwrap().magnitude(), &n("25"));
    assert_eq!(c.subtract(&difference).unwrap().magnitude(), &n("15"));
    assert!(matches!(
        difference.subtract(&c),
        Err(Error::ScaleOperation { .. })
    ));
    assert!(matches!(
        difference.compare(&c),
        Err(Error::ScaleOperation { .. })
    ));
    assert!(matches!(
        db.resolve(&[UnitTerm::new("degC"), UnitTerm::new("m")]),
        Err(Error::AffineComposition)
    ));
    assert_eq!(
        c.divide_by(&n("0")).unwrap_err().result_class(),
        "indeterminate"
    );
}

#[test]
fn aligned_arithmetic_and_structural_units() {
    let db = Register::default();
    let prefixed = |unit, prefix| UnitTerm {
        prefix: Some(prefix),
        ..UnitTerm::new(unit)
    };
    let km = Quantity::new(
        n("1"),
        db.resolve(&[prefixed("m", "k".to_owned())]).unwrap(),
    );
    let metre = q(&db, "1", "m");
    assert_eq!(km.add(&metre).unwrap().magnitude(), &n("1001/1000"));
    assert_eq!(km.ratio(&metre).unwrap(), n("1000"));
    assert_eq!(km.divide(&km).unwrap().unit().terms(), &[]);
    assert_eq!(km.add(&q(&db, "1", "s")), Err(Error::DimensionMismatch));
    let kv = Quantity::new(
        n("10"),
        db.resolve(&[prefixed("V", "k".to_owned())]).unwrap(),
    );
    let power = kv.multiply(&q(&db, "5", "A")).unwrap();
    let w = q(&db, "50000", "W");
    assert_eq!(power.unit().dimension_id(), Some("NISTd16"));
    assert_eq!(power.compare(&w), Ok(Ordering::Equal));
    assert_eq!(power.convert(w.unit()).unwrap(), w);
    assert_ne!(power.compare(&q(&db, "50", "W")), Ok(Ordering::Equal));
}

#[test]
fn exact_powers_and_refusals() {
    let db = Register::default();
    let root = |unit, prefix| UnitTerm {
        power: n("1/2"),
        prefix,
        ..UnitTerm::new(unit)
    };
    let m = db.resolve(&[root("m", None)]).unwrap();
    assert_eq!(m.dimensions()["Length"], n("1/2"));
    assert_eq!(m.dimension_id(), None);
    let cm = db.resolve(&[root("m", Some("c".to_owned()))]).unwrap();
    assert_eq!(cm.factor(), &n("1/10"));
    assert_eq!(
        db.resolve(&[root("m", Some("k".to_owned()))]),
        Err(Error::InexactPower)
    );
    assert!(matches!(
        db.resolve(&[UnitTerm::new("no such unit")]),
        Err(Error::UnknownUnit(_))
    ));
    assert!(matches!(
        db.resolve(&[UnitTerm::new("dB")]),
        Err(Error::UnsupportedScale(_))
    ));
    assert_eq!(n("-1.23e-2"), n("-123/10000"));
    assert!(rational("1/0").is_err());
    assert_eq!(
        db.resolve(&[UnitTerm::new("C")]),
        Err(Error::InconsistentDimensions("NISTu15".into()))
    );
}

#[test]
fn caller_registers_are_independent_and_validated() {
    let mut first = Register::default();
    let mut second = Register::default();
    let id = "NISTu230";
    let dimensions = q(&first, "1", "kg").unit().dimensions().clone();
    let entry = |factor| Registration {
        dimensions: dimensions.clone(),
        factor: n(factor),
        offset: n("0"),
    };
    first.insert(id, entry("50")).unwrap();
    second.insert(id, entry("60")).unwrap();
    assert_eq!(q(&first, "1", id).canonical_magnitude(), n("50"));
    assert_eq!(q(&second, "1", id).canonical_magnitude(), n("60"));
    assert!(matches!(
        first.insert("kg", entry("50")),
        Err(Error::InvalidRegister(_))
    ));
    assert!(matches!(
        first.insert(
            id,
            Registration {
                dimensions: Default::default(),
                ..entry("50")
            }
        ),
        Err(Error::InvalidRegister(_))
    ));
    assert!(matches!(
        first.insert(id, entry("0")),
        Err(Error::InvalidRegister(_))
    ));
}

#[test]
fn structural_json_contract() {
    use serde_json::json;
    use unitsml_rs::evaluate_json;

    let resolved = evaluate_json(br#"{"op":"convert","left":{"value":"20","model":{"unitName":"degC"}},"right":{"terms":[{"unit":"K"}]}}"#);
    assert_eq!(resolved["class"], "value");
    assert_eq!(resolved["value"]["magnitude"], "5863/20");
    assert_eq!(resolved["value"]["dimension_id"], "NISTd5");
    assert_eq!(
        resolved["value"]["terms"],
        json!([{"unit":"NISTu5","prefix":null,"power":"1"}])
    );
    let zero = evaluate_json(br#"{"op":"divide_by","left":{"value":"20"},"scalar":"0"}"#);
    assert_eq!(zero["class"], "indeterminate");
    assert_eq!(zero["error"]["code"], "division_by_zero");
    let unsupported = evaluate_json(br#"{"op":"resolve","left":{"value":"20","expression":"K"}}"#);
    assert_eq!(unsupported["error"]["code"], "invalid_request");
    for input in [".-2", "1_2/3", "1./2"] {
        assert!(rational(input).is_err(), "{input}");
    }
}
