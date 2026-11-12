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
    let shifted = difference.subtract(&c).unwrap();
    assert_eq!(shifted.magnitude(), &n("-5763/20"));
    assert_eq!(shifted.unit(), k.unit());
    assert_eq!(shifted.scale(), Scale::Difference);
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

#[test]
fn registered_identity_precedes_conversion() {
    use unitsml_rs::IdentifiedQuantity;
    let db = Register::default();
    let term = UnitTerm::new("deg");
    let identity = db.identify(std::slice::from_ref(&term)).unwrap();
    assert_eq!(identity.terms()[0].unit, "NISTu149");
    let nominal = IdentifiedQuantity::new(n("3/2"), identity);
    assert_eq!(nominal.magnitude(), &n("3/2"));
    assert_eq!(
        nominal.with_conversions(&db),
        Err(Error::MissingConversion("NISTu149".into()))
    );
    let mut supplied = Register::default();
    supplied
        .insert(
            "deg",
            Registration {
                dimensions: [("PlaneAngle".to_owned(), n("1"))].into(),
                factor: n("1/360"),
                offset: n("0"),
            },
        )
        .unwrap();
    assert_eq!(
        nominal
            .with_conversions(&supplied)
            .unwrap()
            .canonical_magnitude(),
        n("1/240")
    );
    let nominal = unitsml_rs::evaluate_json(
        br#"{"op":"identify","left":{"value":"3/2","terms":[{"unit":"deg"}]}}"#,
    );
    assert_eq!(nominal["value"]["magnitude"], "3/2");
    assert_eq!(nominal["value"]["terms"][0]["unit"], "NISTu149");
}

// Expected values are the output of unitsml-ruby 2ec9583 (unitsdb gem 2.2.15)
// Unitsml::Quantity#+ and #- for every point/difference operand pairing.
#[test]
fn point_and_difference_sums_match_ruby() {
    let db = Register::default();
    let operand = |value, unit, difference| {
        let unit = db.resolve(&[UnitTerm::new(unit)]).unwrap();
        if difference {
            Quantity::difference(n(value), unit)
        } else {
            Quantity::new(n(value), unit)
        }
    };
    #[rustfmt::skip]
    let cases = [
        ("20", "degC", false, '+', "15", "degC", false, "35", "degC", Scale::Interval),
        ("20", "degC", false, '-', "15", "degC", false, "5", "degC", Scale::Difference),
        ("20", "degC", false, '+', "15", "degC", true, "35", "degC", Scale::Interval),
        ("20", "degC", false, '-', "15", "degC", true, "5", "degC", Scale::Interval),
        ("20", "degC", true, '+', "15", "degC", false, "35", "degC", Scale::Interval),
        ("20", "degC", true, '-', "15", "degC", false, "5", "degC", Scale::Difference),
        ("20", "degC", true, '+', "15", "degC", true, "35", "degC", Scale::Difference),
        ("20", "degC", true, '-', "15", "degC", true, "5", "degC", Scale::Difference),
        ("5", "K", false, '+', "20", "degC", false, "5963/20", "K", Scale::Ratio),
        ("5", "K", false, '-', "20", "degC", false, "-5763/20", "K", Scale::Difference),
        ("5", "K", false, '+', "20", "degC", true, "25", "K", Scale::Ratio),
        ("5", "K", false, '-', "20", "degC", true, "-15", "K", Scale::Ratio),
        ("5", "K", true, '+', "20", "degC", false, "5963/20", "K", Scale::Ratio),
        ("5", "K", true, '-', "20", "degC", false, "-5763/20", "K", Scale::Difference),
        ("5", "K", true, '+', "20", "degC", true, "25", "K", Scale::Difference),
        ("5", "K", true, '-', "20", "degC", true, "-15", "K", Scale::Difference),
        ("20", "degC", false, '+', "288.15", "K", false, "35", "degC", Scale::Interval),
        ("20", "degC", false, '-', "288.15", "K", false, "5", "degC", Scale::Difference),
        ("20", "degC", false, '+', "288.15", "K", true, "6163/20", "degC", Scale::Interval),
        ("20", "degC", false, '-', "288.15", "K", true, "-5363/20", "degC", Scale::Interval),
        ("20", "degC", true, '+', "288.15", "K", false, "35", "degC", Scale::Interval),
        ("20", "degC", true, '-', "288.15", "K", false, "5", "degC", Scale::Difference),
        ("20", "degC", true, '+', "288.15", "K", true, "6163/20", "degC", Scale::Difference),
        ("20", "degC", true, '-', "288.15", "K", true, "-5363/20", "degC", Scale::Difference),
    ];
    for (lv, lu, ld, op, rv, ru, rd, magnitude, unit, scale) in cases {
        let (left, right) = (operand(lv, lu, ld), operand(rv, ru, rd));
        let output = match op {
            '+' => left.add(&right),
            _ => left.subtract(&right),
        }
        .unwrap();
        let expected = operand(magnitude, unit, scale == Scale::Difference);
        let case = format!("{lv} {lu} ({ld}) {op} {rv} {ru} ({rd})");
        assert_eq!(
            output.canonical_magnitude(),
            expected.canonical_magnitude(),
            "{case}"
        );
        // Ruby expresses a difference plus a point in the difference's
        // unit; Rust expresses it in the point's unit. Same canonical value.
        if ld && !rd && op == '+' && lu != ru {
            continue;
        }
        assert_eq!(output.magnitude(), &n(magnitude), "{case}");
        assert_eq!(output.unit(), expected.unit(), "{case}");
        assert_eq!(output.scale(), scale, "{case}");
    }
}
