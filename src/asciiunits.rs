use super::{rational, Error, Rational, UnitTerm};
use num_traits::One;
use serde_json::Value;

/// Adapt the AsciiUnits Rust structural model (ABI v1 mode 1 response value).
/// Syntax errors belong to the parser and should be propagated by the caller.
pub fn terms_from_asciiunits(model: &Value) -> Result<Vec<UnitTerm>, Error> {
    let mut terms = Vec::new();
    collect(model, &Rational::one(), 0, &mut terms)?;
    Ok(terms)
}

fn collect(
    node: &Value,
    exponent: &Rational,
    depth: usize,
    terms: &mut Vec<UnitTerm>,
) -> Result<(), Error> {
    if depth >= 128 || terms.len() >= 256 {
        return Err(Error::ResourceLimit);
    }
    let fields = node.as_object().ok_or(Error::UnsupportedOperand)?;
    if let Some(unit) = node["unitName"].as_str() {
        if fields
            .keys()
            .any(|key| !matches!(key.as_str(), "unitName" | "prefix" | "powerNumerator"))
        {
            return Err(Error::UnsupportedOperand);
        }
        let prefix = fields
            .get("prefix")
            .map(|p| {
                p["prefixName"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or(Error::UnsupportedOperand)
            })
            .transpose()?;
        let power = fields
            .get("powerNumerator")
            .map(|p| rational(p["value"].as_str().ok_or(Error::UnsupportedOperand)?))
            .transpose()?
            .unwrap_or_else(Rational::one);
        terms.push(UnitTerm {
            unit: unit.to_owned(),
            prefix,
            power: exponent * power,
        });
        return Ok(());
    }
    if fields.len() == 3 && node["open"] == "(" && node["close"] == ")" {
        return collect(&node["value"], exponent, depth + 1, terms);
    }
    if fields.len() != 1 {
        return Err(Error::UnsupportedOperand);
    }
    match &node["value"] {
        Value::Array(children) if !children.is_empty() => {
            let mut signed = exponent.clone();
            let mut pending_operator = true;
            for child in children {
                match child["text"].as_str() {
                    Some(symbol)
                        if !pending_operator
                            && child.as_object().is_some_and(|obj| obj.len() == 1) =>
                    {
                        match symbol {
                            "/" | "//" => signed = -signed,
                            "*" => {}
                            spaces
                                if !spaces.is_empty()
                                    && spaces.chars().all(char::is_whitespace) => {}
                            _ => return Err(Error::UnsupportedOperand),
                        }
                        pending_operator = true;
                    }
                    Some(_) => return Err(Error::UnsupportedOperand),
                    None => {
                        collect(child, &signed, depth + 1, terms)?;
                        pending_operator = false;
                    }
                }
            }
            if pending_operator {
                return Err(Error::UnsupportedOperand);
            }
            Ok(())
        }
        // The parser's Sqrt model is the sole object-valued single-field node.
        Value::Object(_) => collect(
            &node["value"],
            &(exponent / Rational::from_integer(2.into())),
            depth + 1,
            terms,
        ),
        _ => Err(Error::UnsupportedOperand),
    }
}
