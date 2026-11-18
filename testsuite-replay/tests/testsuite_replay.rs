//! Replays the pinned unitsml-testsuite corpora that unitsml-ruby replays.
//!
//! Set `UNITSML_TESTSUITE_DIR` to a unitsml-testsuite checkout at the commit
//! in unitsml-ruby `spec/fixtures/testsuite-pin.yml`. CI checks it out at
//! that commit. Without the variable the tests skip locally and fail on CI.
//!
//! Textual unit expressions go through the AsciiUnits Rust parser, then the
//! JSON wire entry point, which is the path unitsml-python takes.

use std::collections::BTreeSet;
use std::path::PathBuf;

use asciiunits::Parser;
use serde_json::{json, Value};
use unitsml_rs::{evaluate_json, rational, Rational};

fn corpus(relative: &str) -> Option<Value> {
    let Some(root) = std::env::var_os("UNITSML_TESTSUITE_DIR") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "UNITSML_TESTSUITE_DIR must be set on CI"
        );
        eprintln!("UNITSML_TESTSUITE_DIR unset: skipping {relative}");
        return None;
    };
    let path = PathBuf::from(root).join(relative);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    Some(serde_json::from_str(&text).unwrap())
}

struct Replay {
    parser: Parser,
}

impl Replay {
    fn new() -> Self {
        Self {
            parser: Parser::new().expect("AsciiUnits artifact"),
        }
    }

    fn operand(&self, value: Option<&str>, expression: Option<&str>) -> Value {
        let mut operand = json!({});
        if let Some(value) = value {
            operand["value"] = json!(value);
        }
        if let Some(expression) = expression {
            operand["model"] = self.parser.parse(expression).expect(expression);
        }
        operand
    }

    fn wire(&self, request: Value) -> Value {
        evaluate_json(request.to_string().as_bytes())
    }

    fn value(&self, request: Value) -> Value {
        let reply = self.wire(request.clone());
        assert_eq!(reply["class"], "value", "{request} -> {reply}");
        reply["value"].clone()
    }

    fn refusal(&self, request: Value) -> (String, String) {
        let reply = self.wire(request.clone());
        assert_ne!(reply["class"], "value", "{request} -> {reply}");
        (
            reply["class"].as_str().unwrap().to_owned(),
            reply["error"]["code"].as_str().unwrap().to_owned(),
        )
    }

    fn resolved_terms(&self, unit: &str) -> Value {
        self.value(json!({"op":"resolve","left":self.operand(Some("1"), Some(unit))}))["terms"]
            .clone()
    }
}

fn exact(text: &str) -> Rational {
    rational(text).unwrap_or_else(|_| panic!("not an exact number: {text}"))
}

fn result_operand(value: &Value) -> Value {
    json!({
        "value": value["magnitude"],
        "terms": value["terms"],
        "difference": value["scale"] == "difference",
    })
}

fn pair(given: &Value) -> (&str, &str) {
    (given[0].as_str().unwrap(), given[1].as_str().unwrap())
}

/// F40 result: exact rational, dimension id and vector, scale, target terms.
fn assert_quantity(
    replay: &Replay,
    id: &str,
    result: &Value,
    expected: &Value,
    unit: Option<&str>,
    scale: Option<&str>,
) {
    let want = expected["value"]["rational"].as_str().unwrap();
    assert_eq!(
        exact(result["magnitude"].as_str().unwrap()),
        exact(want),
        "{id}"
    );
    assert_eq!(result["dimension_id"], expected["dimension_id"], "{id}");
    let dimensions: serde_json::Map<String, Value> = expected["dimension_vector"]
        .as_object()
        .map(|vector| {
            vector
                .iter()
                .map(|(name, power)| (name.clone(), json!(power.to_string())))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(result["dimensions"], Value::Object(dimensions), "{id}");
    let want_scale = scale.unwrap_or_else(|| expected["scale"].as_str().unwrap());
    assert_eq!(result["scale"], want_scale, "{id}");
    match unit {
        None => assert_eq!(result["terms"], json!([]), "{id}"),
        Some(unit) => assert_eq!(result["terms"], replay.resolved_terms(unit), "{id}"),
    }
}

#[test]
fn f40_unitsml_quantities_match_ruby() {
    let Some(data) = corpus("corpora/features/unitsml-quantities.json") else {
        return;
    };
    let replay = Replay::new();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 39);
    for row in cases {
        let id = row["id"].as_str().unwrap();
        let given = &row["input"];
        let expected = &row["result"];
        let text = |key: &str| given[key].as_str();
        match row["kind"].as_str().unwrap() {
            "convert" => {
                let result = replay.value(json!({
                    "op": "convert",
                    "left": replay.operand(text("value"), text("unit")),
                    "right": replay.operand(None, text("target")),
                }));
                assert_quantity(&replay, id, &result, expected, text("target"), None);
            }
            "dimension" => {
                let value = text("value").or(Some("1"));
                let result = replay
                    .value(json!({"op":"resolve","left":replay.operand(value, text("unit"))}));
                if id == "dim-1" {
                    // The historical Ruby call crashed; the structural profile has a dimensionless unit.
                    assert_eq!(row["error_class"], "NoMethodError");
                    assert_eq!(
                        result,
                        json!({
                            "magnitude": "1", "canonical_magnitude": "1", "dimension_id": null,
                            "dimensions": {}, "scale": "ratio",
                            "terms": [{"unit": "NISTu61", "prefix": null, "power": "1"}],
                        })
                    );
                } else {
                    assert_quantity(&replay, id, &result, expected, text("unit"), None);
                }
            }
            "difference" => {
                let (high, high_unit) = pair(&given["high"]);
                let (low, low_unit) = pair(&given["low"]);
                let delta = replay.value(json!({
                    "op": "subtract",
                    "left": replay.operand(Some(high), Some(high_unit)),
                    "right": replay.operand(Some(low), Some(low_unit)),
                }));
                let converted = replay.value(json!({
                    "op": "convert",
                    "left": result_operand(&delta),
                    "right": replay.operand(None, text("target")),
                }));
                // The source records lost scale typing; the exact difference survives conversion.
                assert_eq!(expected["scale"], "ratio");
                assert_quantity(
                    &replay,
                    id,
                    &converted,
                    expected,
                    text("target"),
                    Some("difference"),
                );
            }
            "product" => {
                let (lv, lu) = pair(&given["left"]);
                let (rv, ru) = pair(&given["right"]);
                let product = replay.value(json!({
                    "op": "multiply",
                    "left": replay.operand(Some(lv), Some(lu)),
                    "right": replay.operand(Some(rv), Some(ru)),
                }));
                let unit = expected["product"]["unit"].as_str();
                assert_quantity(&replay, id, &product, &expected["product"], unit, None);
                for (field, unit) in [("to_W", "W"), ("to_kW", "kW")] {
                    let converted = replay.value(json!({
                        "op": "convert",
                        "left": result_operand(&product),
                        "right": replay.operand(None, Some(unit)),
                    }));
                    assert_quantity(&replay, id, &converted, &expected[field], Some(unit), None);
                }
                assert_eq!(expected["equals_50000_W"], false);
                assert_eq!(expected["equals_50_kW"], true);
                // Physical comparison uses the same exact canonical magnitude in both units.
                for (value, unit) in [("50000", "W"), ("50", "kW")] {
                    let order = replay.value(json!({
                        "op": "compare",
                        "left": result_operand(&product),
                        "right": replay.operand(Some(value), Some(unit)),
                    }));
                    assert_eq!(order, 0, "{id} vs {value} {unit}");
                }
            }
            "equality" => {
                assert_eq!(given, &json!({"left": "10 kV * 5 A", "right": "50 W"}));
                assert_eq!(expected["equal"], true);
                let product = replay.value(json!({
                    "op": "multiply",
                    "left": replay.operand(Some("10"), Some("kV")),
                    "right": replay.operand(Some("5"), Some("A")),
                }));
                let order = replay.value(json!({
                    "op": "compare",
                    "left": result_operand(&product),
                    "right": replay.operand(Some("50"), Some("W")),
                }));
                assert_eq!(order, 1, "{id}");
            }
            "refusal" => f40_refusal(&replay, id, row["error_class"].as_str().unwrap()),
            other => panic!("unsupported F40 case kind {other}"),
        }
    }
}

fn f40_refusal(replay: &Replay, id: &str, source_error: &str) {
    if id == "refuse-unknown-unit" {
        assert_eq!(source_error, "Unitsml::Errors::ParseError");
        assert!(replay.parser.parse("notaunit_xyz").is_err());
        return;
    }
    let o = |value: &str, unit: &str| replay.operand(Some(value), Some(unit));
    let target = |unit: &str| replay.operand(None, Some(unit));
    let (request, code, source_suffix) = match id {
        "refuse-degc2-to-k2" => (
            json!({"op":"convert","left":o("1","degC^2"),"right":target("K^2")}),
            "affine_composition",
            "ConversionError",
        ),
        "refuse-m-to-s" => (
            json!({"op":"convert","left":o("1","m"),"right":target("s")}),
            "dimension_mismatch",
            "DimensionMismatchError",
        ),
        "refuse-degc-to-k2" => (
            json!({"op":"convert","left":o("1","degC"),"right":target("K^2")}),
            "dimension_mismatch",
            "DimensionMismatchError",
        ),
        "refuse-v-plus-s" => (
            json!({"op":"add","left":o("1","V"),"right":o("1","s")}),
            "dimension_mismatch",
            "DimensionMismatchError",
        ),
        "refuse-degc-times-2" => (
            json!({"op":"scale","left":o("10","degC"),"scalar":"2"}),
            "scale_operation",
            "ScaleOperationError",
        ),
        "refuse-degc-times-degc" => (
            json!({"op":"multiply","left":o("10","degC"),"right":o("2","degC")}),
            "scale_operation",
            "ScaleOperationError",
        ),
        "refuse-div-zero-quantity" => (
            json!({"op":"divide","left":o("1","m"),"right":o("0","s")}),
            "division_by_zero",
            "IndeterminateResultError",
        ),
        "refuse-div-zero-scalar" => (
            json!({"op":"divide_by","left":o("1","m"),"scalar":"0"}),
            "division_by_zero",
            "IndeterminateResultError",
        ),
        "refuse-both-keywords" => (
            json!({"op":"resolve","left":{"value":"1","model":{"unitName":"m"},"terms":[]}}),
            "invalid_request",
            "ArgumentError",
        ),
        other => panic!("unlisted F40 refusal {other}"),
    };
    assert_eq!(source_error.rsplit("::").next().unwrap(), source_suffix);
    let class = if code == "division_by_zero" {
        "indeterminate"
    } else {
        "error"
    };
    assert_eq!(
        replay.refusal(request),
        (class.to_owned(), code.to_owned()),
        "{id}"
    );
}

/// Rows this crate cannot replay, with the reason. The crate has no XML
/// conversion-definition interpreter (see README), so every row whose
/// expectation depends on executing or resolving a `*ConversionFrom`
/// element is refused here rather than replayed against canonical data.
const CONVERSION_REFUSALS: &[(&str, &str)] = &[
    (
        "degc-to-k-definition",
        "executes a Float64ConversionFrom definition",
    ),
    (
        "degc-to-degf-definition",
        "executes a Float64ConversionFrom definition",
    ),
    (
        "degc-100-to-degf-definition",
        "executes a Float64ConversionFrom definition",
    ),
    (
        "diff-degc-to-k-definition",
        "executes a Float64ConversionFrom definition",
    ),
    (
        "def-degc-forward-inverse-exact",
        "XML conversion-definition interpreter",
    ),
    (
        "def-degf-five-ninths-exact",
        "XML conversion-definition interpreter",
    ),
    (
        "def-percent-to-one-exact",
        "XML conversion-definition interpreter",
    ),
    (
        "def-non-exact-preserves-precision",
        "XML conversion-definition interpreter",
    ),
    (
        "resolve-document-local-fragment",
        "resolves an XML initialUnit reference",
    ),
    (
        "resolve-register-fragment-fallback",
        "resolves an XML initialUnit reference",
    ),
    (
        "resolve-reference-uri",
        "resolves an XML initialUnit reference",
    ),
    (
        "err-affine-definition-at-power",
        "refusal comes from a conversion definition",
    ),
    (
        "err-definition-disagrees-with-canonical",
        "checks a definition against canonical data",
    ),
    (
        "err-unresolvable-initial-unit",
        "resolves an XML initialUnit reference",
    ),
    (
        "err-special-conversion-refused",
        "classifies a SpecialConversionFrom element",
    ),
    (
        "err-wsdl-conversion-refused",
        "classifies a WSDLConversionFrom element",
    ),
];

#[test]
fn conversion_corpus_matches_ruby() {
    let Some(data) = corpus("corpora/conversions/expected-conversions.json") else {
        return;
    };
    let replay = Replay::new();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 25);
    let refused: BTreeSet<&str> = CONVERSION_REFUSALS.iter().map(|(id, _)| *id).collect();
    assert_eq!(refused.len(), CONVERSION_REFUSALS.len());
    let mut replayed = BTreeSet::new();
    for row in cases {
        let id = row["id"].as_str().unwrap();
        if refused.contains(id) {
            continue;
        }
        let text = |key: &str| row[key].as_str();
        let target = replay.operand(None, text("target"));
        let result = match row["kind"].as_str().unwrap() {
            "convert" => replay.value(json!({
                "op": "convert",
                "left": replay.operand(text("value"), text("unit_expression")),
                "right": target,
            })),
            "difference" => {
                let delta = replay.value(json!({
                    "op": "subtract",
                    "left": replay.operand(text("value"), text("unit_expression")),
                    "right": replay.operand(text("other_value"), text("other_unit_expression")),
                }));
                let converted = replay.value(json!({
                    "op": "convert",
                    "left": result_operand(&delta),
                    "right": target,
                }));
                assert_eq!(converted["scale"], "difference", "{id}");
                converted
            }
            other => panic!("{id}: unlisted {other} row needs a replay or a listed refusal"),
        };
        assert!(row.get("definitions").is_none(), "{id}");
        assert_eq!(
            exact(result["magnitude"].as_str().unwrap()),
            exact(text("expected").unwrap()),
            "{id}"
        );
        assert_eq!(
            result["terms"],
            replay.resolved_terms(text("target").unwrap()),
            "{id}"
        );
        replayed.insert(id);
    }
    let all: BTreeSet<&str> = cases
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert!(refused.is_subset(&all), "stale refusal ids");
    assert_eq!(replayed.len() + refused.len(), all.len());
    assert_eq!(replayed.len(), 9);
}
