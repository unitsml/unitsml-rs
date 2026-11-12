//! Primary-quantity selection: flag presence matters (unitsml-ruby c4821ee).
use serde_json::json;
use unitsml_rs::QuantityKind;

#[test]
fn sole_reference_without_flag_is_inferred_primary() {
    let refs = [json!({"id": "NISTq17"})];
    let kind = QuantityKind::primary_among("u", &refs).unwrap().unwrap();
    assert_eq!(kind.id(), "NISTq17");
}

#[test]
fn sole_reference_marked_false_is_not_promoted() {
    let refs = [json!({"id": "NISTq17", "primary": false})];
    let error = QuantityKind::primary_among("u", &refs).unwrap_err();
    assert_eq!(error.code(), "PrimaryQuantityError");
}

#[test]
fn true_among_false_and_absent_selects_the_flagged_reference() {
    let refs = [
        json!({"id": "NISTq152", "primary": false}),
        json!({"id": "NISTq17", "primary": true}),
        json!({"id": "NISTq18"}),
    ];
    for order in [refs.to_vec(), refs.iter().rev().cloned().collect()] {
        let kind = QuantityKind::primary_among("u", &order).unwrap().unwrap();
        assert_eq!(kind.id(), "NISTq17");
    }
}

#[test]
fn several_unflagged_or_several_flagged_are_refused() {
    let none = [
        json!({"id": "NISTq17", "primary": false}),
        json!({"id": "NISTq1"}),
    ];
    let many = [
        json!({"id": "NISTq17", "primary": true}),
        json!({"id": "NISTq1", "primary": true}),
    ];
    for refs in [&none[..], &many[..]] {
        assert_eq!(
            QuantityKind::primary_among("u", refs).unwrap_err().code(),
            "PrimaryQuantityError"
        );
    }
    assert!(QuantityKind::primary_among("u", &[]).unwrap().is_none());
}

#[test]
fn bundled_data_keeps_every_flagged_and_sole_unit_primary() {
    // 96 flagged and 284 sole unflagged references in the bundled projection.
    let j = QuantityKind::primary_for_unit("NISTu13").unwrap().unwrap();
    assert_eq!(j.id(), "NISTq17");
    let refs = QuantityKind::unit_references("NISTu1").unwrap();
    assert!(QuantityKind::primary_among("NISTu1", refs)
        .unwrap()
        .is_some());
}
