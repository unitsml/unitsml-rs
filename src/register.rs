use super::{rational, Dimensions, Error, IdentifiedUnit, Rational, Scale, Unit, UnitTerm};
use num_traits::{One, Signed, ToPrimitive, Zero};
use serde_json::Value;
use std::collections::BTreeMap;
use unitsdb_rs::{find, records, Collection, Lookup};

type Record = &'static Value;

#[derive(Debug, Clone)]
pub struct Registration {
    pub dimensions: Dimensions,
    pub factor: Rational,
    pub offset: Rational,
}

/// Local conversion overrides validated against the pinned UnitsDB structure.
#[derive(Debug, Default)]
pub struct Register {
    entries: BTreeMap<String, Registration>,
}

impl Register {
    pub fn insert(&mut self, identity: &str, entry: Registration) -> Result<(), Error> {
        let record = unit(identity)?;
        let id = record_id(record)?;
        let scale = scale(record)?;
        let dimensions = dimensions(record, &Rational::one(), &mut Vec::new())?;
        if entry.factor <= Rational::zero()
            || entry.dimensions != dimensions
            || (scale != Scale::Interval && !entry.offset.is_zero())
        {
            return Err(Error::InvalidRegister(id.to_owned()));
        }
        if let Some((factor, offset)) = canonical(record)? {
            let tolerance = Rational::from_integer(1_000_000_000.into());
            if (&factor - &entry.factor).abs() > factor.abs() / &tolerance
                || (&offset - &entry.offset).abs() > offset.abs() / tolerance
            {
                return Err(Error::InvalidRegister(id.to_owned()));
            }
        }
        self.entries.insert(id.to_owned(), entry);
        Ok(())
    }

    pub fn identify(&self, terms: &[UnitTerm]) -> Result<IdentifiedUnit, Error> {
        if terms.len() > 256 {
            return Err(Error::ResourceLimit);
        }
        let terms = terms
            .iter()
            .map(|term| {
                let unit = record_id(unit(&term.unit)?)?.to_owned();
                let prefix = term
                    .prefix
                    .as_deref()
                    .map(prefix)
                    .transpose()?
                    .map(record_id)
                    .transpose()?
                    .map(str::to_owned);
                Ok(UnitTerm {
                    unit,
                    prefix,
                    power: term.power.clone(),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(IdentifiedUnit { terms })
    }

    pub fn resolve(&self, terms: &[UnitTerm]) -> Result<Unit, Error> {
        self.resolve_identified(&self.identify(terms)?)
    }

    pub(crate) fn resolve_identified(&self, unit_identity: &IdentifiedUnit) -> Result<Unit, Error> {
        let terms = unit_identity.terms();
        let mut result = Unit::identity();
        for term in terms {
            let record = unit(&term.unit)?;
            let scale = scale(record)?;
            let prefix = prefix_factor(term.prefix.as_deref())?;
            if scale == Scale::Interval
                && (terms.len() != 1 || !term.power.is_one() || !prefix.is_one())
            {
                return Err(Error::AffineComposition);
            }
            let dimensions = dimensions(record, &term.power, &mut Vec::new())?;
            let (factor, offset) = self.affine(record, &mut Vec::new())?;
            if !offset.is_zero() && (terms.len() != 1 || !term.power.is_one()) {
                return Err(Error::AffineComposition);
            }
            let leaf = Unit {
                terms: vec![term.clone()],
                dimensions,
                factor: pow_exact(&(factor * prefix), &term.power)?,
                offset,
                scale,
            };
            if terms.len() == 1 {
                return Ok(leaf);
            }
            result = result.combine(&leaf, false);
        }
        Ok(result)
    }

    fn affine(
        &self,
        record: Record,
        seen: &mut Vec<&'static str>,
    ) -> Result<(Rational, Rational), Error> {
        let id = record_id(record)?;
        if let Some(entry) = self.entries.get(id) {
            return Ok((entry.factor.clone(), entry.offset.clone()));
        }
        if let Some(affine) = canonical(record)? {
            return Ok(affine);
        }
        if (base_dimension(id).is_some() && id != "NISTu27") || id == "NISTu61" {
            return Ok((Rational::one(), Rational::zero()));
        }
        visit(id, seen, |seen| {
            let mut factor = Rational::one();
            for entry in chain(record).ok_or_else(|| Error::MissingConversion(id.to_owned()))? {
                let target = target(entry)?;
                let (base, offset) = self.affine(target, seen)?;
                if !offset.is_zero() {
                    return Err(Error::AffineComposition);
                }
                let prefix = prefix_factor(
                    entry
                        .pointer("/prefix_reference/id")
                        .and_then(Value::as_str),
                )?;
                factor *= pow_exact(&(base * prefix), &power(entry)?)?;
            }
            Ok((factor, Rational::zero()))
        })
    }
}

fn unit(key: &str) -> Result<Record, Error> {
    find(Lookup::UnitId, key)
        .or_else(|| find(Lookup::UnitSymbol, key))
        .or_else(|| find(Lookup::UnitUri, key))
        .ok_or_else(|| Error::UnknownUnit(key.to_owned()))
}

fn record_id(record: Record) -> Result<&'static str, Error> {
    record
        .pointer("/identifiers/0/id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRegister("missing unit ID".to_owned()))
}

fn scale(record: Record) -> Result<Scale, Error> {
    let id = record
        .pointer("/scale_reference/id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::UnsupportedScale("missing scale reference".to_owned()))?;
    let properties = &find(Lookup::ScaleId, id)
        .ok_or_else(|| Error::UnsupportedScale(id.to_owned()))?["properties"];
    if properties["continuous"] != true || properties["logarithmic"] != false {
        return Err(Error::UnsupportedScale(id.to_owned()));
    }
    if properties["ratio"] == true {
        Ok(Scale::Ratio)
    } else if properties["interval"] == true {
        Ok(Scale::Interval)
    } else {
        Err(Error::UnsupportedScale(id.to_owned()))
    }
}

fn canonical(record: Record) -> Result<Option<(Rational, Rational)>, Error> {
    let Some(conversion) = record
        .get("canonical_conversion")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    let factor = conversion["factor"]
        .as_str()
        .ok_or_else(|| Error::InvalidRegister("missing factor".to_owned()))?;
    let factor = rational(factor)?;
    if factor <= Rational::zero() {
        return Err(Error::InvalidRegister(record_id(record)?.to_owned()));
    }
    let offset = rational(conversion["offset"].as_str().unwrap_or("0"))?;
    Ok(Some((factor, offset)))
}

fn chain(record: Record) -> Option<&'static [Value]> {
    ["si_derived_bases", "root_units"]
        .into_iter()
        .find_map(|key| {
            record[key]
                .as_array()
                .filter(|list| !list.is_empty())
                .map(Vec::as_slice)
        })
}

fn target(entry: &Value) -> Result<Record, Error> {
    let id = entry
        .pointer("/unit_reference/id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRegister("missing conversion target".to_owned()))?;
    unit(id)
}

fn power(entry: &Value) -> Result<Rational, Error> {
    match &entry["power"] {
        Value::Null => Ok(Rational::one()),
        Value::String(s) => rational(s),
        value => rational(&value.to_string()),
    }
}

fn prefix(key: &str) -> Result<Record, Error> {
    find(Lookup::PrefixId, key)
        .or_else(|| find(Lookup::PrefixSymbol, key))
        .ok_or_else(|| Error::UnknownPrefix(key.to_owned()))
}

fn prefix_power(record: Record) -> Result<Rational, Error> {
    let base = rational(&record["base"].to_string())?;
    if base <= Rational::zero() {
        return Err(Error::InvalidRegister(record_id(record)?.to_owned()));
    }
    pow_exact(&base, &power(record)?)
}

fn prefix_factor(key: Option<&str>) -> Result<Rational, Error> {
    match key {
        Some(key) => prefix_power(prefix(key)?),
        None => Ok(Rational::one()),
    }
}

fn base_dimension(id: &str) -> Option<&'static str> {
    match id {
        "NISTu1" => Some("Length"),
        "NISTu2" | "NISTu27" => Some("Mass"),
        "NISTu3" => Some("Time"),
        "NISTu4" => Some("ElectricCurrent"),
        "NISTu5" => Some("ThermodynamicTemperature"),
        "NISTu6" => Some("AmountOfSubstance"),
        "NISTu7" => Some("LuminousIntensity"),
        _ => None,
    }
}

fn dimensions(
    record: Record,
    exponent: &Rational,
    seen: &mut Vec<&'static str>,
) -> Result<Dimensions, Error> {
    let id = record_id(record)?;
    if let Some(name) = base_dimension(id) {
        if exponent.is_zero() {
            return Ok(Dimensions::new());
        }
        return Ok(BTreeMap::from([(name.to_owned(), exponent.clone())]));
    }
    let entries = record
        .pointer("/canonical_conversion/root_units")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .or_else(|| chain(record));
    let Some(entries) = entries else {
        return Ok(scaled_dimensions(quantity_dimensions(record)?, exponent));
    };
    let dimensions = visit(id, seen, |seen| {
        let mut result = Dimensions::new();
        for entry in entries {
            let exponent = exponent * power(entry)?;
            for (name, sub) in dimensions(target(entry)?, &exponent, seen)? {
                *result.entry(name).or_default() += sub;
            }
        }
        result.retain(|_, power| !power.is_zero());
        Ok(result)
    })?;
    let primary = record["quantity_references"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["primary"] == true));
    if primary && dimensions != scaled_dimensions(quantity_dimensions(record)?, exponent) {
        return Err(Error::InconsistentDimensions(id.to_owned()));
    }
    Ok(dimensions)
}

fn scaled_dimensions(mut dimensions: Dimensions, exponent: &Rational) -> Dimensions {
    for power in dimensions.values_mut() {
        *power *= exponent;
    }
    dimensions.retain(|_, power| !power.is_zero());
    dimensions
}

fn visit<T>(
    id: &'static str,
    seen: &mut Vec<&'static str>,
    f: impl FnOnce(&mut Vec<&'static str>) -> Result<T, Error>,
) -> Result<T, Error> {
    if seen.len() >= 64 || seen.contains(&id) {
        return Err(Error::InvalidRegister(id.to_owned()));
    }
    seen.push(id);
    let result = f(seen);
    seen.pop();
    result
}

fn pow_exact(base: &Rational, exponent: &Rational) -> Result<Rational, Error> {
    let numerator = exponent
        .numer()
        .to_i32()
        .filter(|n| n.unsigned_abs() <= 4096)
        .ok_or(Error::ResourceLimit)?;
    let root = exponent
        .denom()
        .to_u32()
        .filter(|n| *n <= 4096)
        .ok_or(Error::ResourceLimit)?;
    if root == 1 {
        return Ok(base.pow(numerator));
    }
    let top = base.numer().nth_root(root);
    let bottom = base.denom().nth_root(root);
    if top.pow(root) != *base.numer() || bottom.pow(root) != *base.denom() {
        return Err(Error::InexactPower);
    }
    Ok(Rational::new(top, bottom).pow(numerator))
}

pub(crate) fn dimension_id(dimensions: &Dimensions) -> Option<&'static str> {
    if dimensions.is_empty() || dimensions.values().any(|power| !power.is_integer()) {
        return None;
    }
    records(Collection::Dimensions).iter().find(|record| {
        COMPONENTS.iter().all(|(name, key)| {
            let power = record[*key]["power"].as_i64().unwrap_or(0);
            let power = Rational::from_integer(power.into());
            dimensions
                .get(*name)
                .map_or_else(|| power.is_zero(), |expected| expected == &power)
        })
    })?["id"]
        .as_str()
}

const COMPONENTS: &[(&str, &str)] = &[
    ("Length", "length"),
    ("Mass", "mass"),
    ("Time", "time"),
    ("ElectricCurrent", "electric_current"),
    ("ThermodynamicTemperature", "thermodynamic_temperature"),
    ("AmountOfSubstance", "amount_of_substance"),
    ("LuminousIntensity", "luminous_intensity"),
    ("PlaneAngle", "plane_angle"),
];

fn quantity_dimensions(record: Record) -> Result<Dimensions, Error> {
    let id = record_id(record)?;
    let missing = || Error::MissingConversion(id.to_owned());
    let refs = record["quantity_references"]
        .as_array()
        .ok_or_else(missing)?;
    let use_primary = refs.iter().any(|item| item["primary"] == true);
    let mut vectors = refs
        .iter()
        .filter(|item| !use_primary || item["primary"] == true)
        .map(|item| {
            let quantity = item["id"]
                .as_str()
                .and_then(|key| find(Lookup::QuantityId, key))
                .ok_or_else(missing)?;
            let dim = quantity
                .pointer("/dimension_reference/id")
                .and_then(Value::as_str)
                .and_then(|key| find(Lookup::DimensionId, key))
                .ok_or_else(missing)?;
            COMPONENTS
                .iter()
                .filter(|(_, key)| !dim[*key].is_null())
                .map(|(name, key)| Ok(((*name).to_owned(), power(&dim[*key])?)))
                .collect::<Result<Dimensions, Error>>()
        });
    let first = vectors.next().ok_or_else(missing)??;
    for vector in vectors {
        if vector? != first {
            return Err(missing());
        }
    }
    Ok(first)
}
