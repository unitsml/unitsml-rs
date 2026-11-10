use crate::{
    rational, terms_from_asciiunits, Error, Quantity, Rational, Register, Registration, Scale,
    Unit, UnitTerm,
};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;

/// Evaluate the version-1 structural JSON request. All numeric fields use
/// exact strings. The result class is value, indeterminate, or error.
pub fn evaluate_json(input: &[u8]) -> Value {
    if input.len() > 1024 * 1024 {
        return refusal("error", "resource_limit", "request exceeds 1 MiB");
    }
    let result = serde_json::from_slice::<Value>(input)
        .map_err(|error| WireError::Invalid(error.to_string()))
        .and_then(|value| evaluate(&value));
    match result {
        Ok(value) => json!({"abi_version":1,"class":"value","value":value}),
        Err(WireError::Algebra(error)) => {
            refusal(error.result_class(), code(&error), &error.to_string())
        }
        Err(WireError::Invalid(detail)) => refusal("error", "invalid_request", &detail),
    }
}

pub(crate) fn refusal(class: &str, code: &str, detail: &str) -> Value {
    json!({"abi_version":1,"class":class,"error":{"code":code,"detail":detail}})
}

enum WireError {
    Algebra(Error),
    Invalid(String),
}

impl From<Error> for WireError {
    fn from(value: Error) -> Self {
        Self::Algebra(value)
    }
}

fn invalid(field: &str) -> WireError {
    WireError::Invalid(format!("invalid or missing {field}"))
}

fn fields(value: &Value, allowed: &[&str], field: &str) -> Result<(), WireError> {
    let object = value.as_object().ok_or_else(|| invalid(field))?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(field));
    }
    Ok(())
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, WireError> {
    value.as_str().ok_or_else(|| invalid(field))
}

fn number(value: &Value, field: &str) -> Result<Rational, WireError> {
    Ok(rational(text(value, field)?)?)
}

fn evaluate(request: &Value) -> Result<Value, WireError> {
    fields(
        request,
        &["op", "left", "right", "scalar", "register"],
        "request fields",
    )?;
    let op = text(&request["op"], "op")?;
    let mut db = Register::default();
    if let Some(entries) = request.get("register") {
        let entries = entries.as_array().ok_or_else(|| invalid("register"))?;
        if entries.len() > 256 {
            return Err(Error::ResourceLimit.into());
        }
        for entry in entries {
            fields(
                entry,
                &["unit", "dimensions", "factor", "offset"],
                "register entry fields",
            )?;
            let id = text(&entry["unit"], "register.unit")?;
            let dimensions = entry["dimensions"]
                .as_object()
                .ok_or_else(|| invalid("register.dimensions"))?;
            let dimensions = dimensions
                .iter()
                .map(|(name, value)| {
                    Ok((name.clone(), number(value, "register.dimensions exponent")?))
                })
                .collect::<Result<_, WireError>>()?;
            let factor = number(&entry["factor"], "register.factor")?;
            let offset = entry
                .get("offset")
                .map(|value| number(value, "register.offset"))
                .transpose()?
                .unwrap_or_default();
            db.insert(
                id,
                Registration {
                    dimensions,
                    factor,
                    offset,
                },
            )?;
        }
    }
    let left = quantity(&db, &request["left"])?;
    let right = || quantity(&db, &request["right"]);
    let scalar = || number(&request["scalar"], "scalar");
    let output = match op {
        "resolve" => left,
        "convert" => left.convert(&unit(&db, &request["right"])?)?,
        "add" => left.add(&right()?)?,
        "subtract" => left.subtract(&right()?)?,
        "multiply" => left.multiply(&right()?)?,
        "divide" => left.divide(&right()?)?,
        "scale" => left.scale_by(&scalar()?)?,
        "divide_by" => left.divide_by(&scalar()?)?,
        "compare" => {
            let order = match left.compare(&right()?)? {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            };
            return Ok(json!(order));
        }
        "ratio" => return Ok(json!(left.ratio(&right()?)?.to_string())),
        _ => return Err(invalid("op")),
    };
    let dimensions: Map<_, _> = output
        .unit()
        .dimensions()
        .iter()
        .map(|(name, power)| (name.clone(), json!(power.to_string())))
        .collect();
    let scale = match output.scale() {
        Scale::Ratio => "ratio",
        Scale::Interval => "interval",
        Scale::Difference => "difference",
    };
    let terms: Vec<_> = output
        .unit()
        .terms()
        .iter()
        .map(|term| json!({"unit":term.unit,"prefix":term.prefix,"power":term.power.to_string()}))
        .collect();
    Ok(json!({
        "magnitude": output.magnitude().to_string(),
        "canonical_magnitude": output.canonical_magnitude().to_string(),
        "dimension_id": output.unit().dimension_id(),
        "dimensions": dimensions,
        "scale": scale,
        "terms": terms,
    }))
}

fn quantity(db: &Register, value: &Value) -> Result<Quantity, WireError> {
    let unit = unit(db, value)?;
    let magnitude = number(&value["value"], "operand.value")?;
    let difference = value
        .get("difference")
        .map(|value| value.as_bool().ok_or_else(|| invalid("operand.difference")))
        .transpose()?
        .unwrap_or(false);
    Ok(if difference {
        Quantity::difference(magnitude, unit)
    } else {
        Quantity::new(magnitude, unit)
    })
}

fn unit(db: &Register, value: &Value) -> Result<Unit, WireError> {
    fields(
        value,
        &["value", "terms", "model", "difference"],
        "operand fields",
    )?;
    let terms = match (value.get("model"), value.get("terms")) {
        (Some(_), Some(_)) => return Err(invalid("operand has both model and terms")),
        (Some(model), None) => terms_from_asciiunits(model)?,
        (None, Some(terms)) => {
            let terms = terms.as_array().ok_or_else(|| invalid("operand.terms"))?;
            if terms.len() > 256 {
                return Err(Error::ResourceLimit.into());
            }
            terms
                .iter()
                .map(|term| {
                    fields(term, &["unit", "prefix", "power"], "unit term fields")?;
                    let mut output = UnitTerm::new(text(&term["unit"], "term.unit")?);
                    if let Some(prefix) = term.get("prefix").filter(|value| !value.is_null()) {
                        output.prefix = Some(text(prefix, "term.prefix")?.to_owned());
                    }
                    if let Some(power) = term.get("power") {
                        output.power = number(power, "term.power")?;
                    }
                    Ok(output)
                })
                .collect::<Result<_, WireError>>()?
        }
        (None, None) => Vec::new(),
    };
    Ok(db.resolve(&terms)?)
}

fn code(error: &Error) -> &'static str {
    match error {
        Error::UnsupportedOperand => "unsupported_operand",
        Error::UnknownUnit(_) => "unknown_unit",
        Error::UnknownPrefix(_) => "unknown_prefix",
        Error::MissingConversion(_) => "missing_conversion",
        Error::InvalidRegister(_) => "invalid_register",
        Error::InconsistentDimensions(_) => "inconsistent_dimensions",
        Error::UnsupportedScale(_) => "unsupported_scale",
        Error::ScaleOperation { .. } => "scale_operation",
        Error::DimensionMismatch => "dimension_mismatch",
        Error::AffineComposition => "affine_composition",
        Error::InexactPower => "inexact_power",
        Error::InvalidNumber(_) => "invalid_number",
        Error::ResourceLimit => "resource_limit",
        Error::DivisionByZero => "division_by_zero",
    }
}
