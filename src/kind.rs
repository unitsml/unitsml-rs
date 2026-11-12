//! Data-declared quantity kinds (F40), ported from unitsml-ruby
//! `Unitsml::QuantityKind` at claude/f40-ruby-features-20261111 (2337a12).
//!
//! A kind is a UnitsDB quantity record. Two kinds are interchangeable when
//! they are the same record or share a non-null `equivalence_class`
//! (UnitsDB quantities-schema.yaml:47-49). The primary quantity of a unit is
//! the single `quantity_references` entry flagged `primary: true`
//! (units-schema.yaml:147-149); data order never selects it.
use serde_json::Value;
use std::fmt;
use unitsdb_rs::{find, records, Collection, Lookup};

/// Kind errors, kept apart from [`crate::Error`] so existing exhaustive
/// matches over unit errors are unchanged.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum KindError {
    /// No UnitsDB quantity has this identifier or name.
    Unknown(String),
    /// The unit's data flags no primary among several references, or several.
    Primary(String),
    /// The kind's dimension differs from the unit's dimension.
    Dimension { kind: String, unit: String },
    /// Neither the same quantity nor a shared equivalence class.
    Mismatch { left: String, right: String },
}

impl fmt::Display for KindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(kind) => write!(f, "unknown quantity kind {kind:?}"),
            Self::Primary(detail) => f.write_str(detail),
            Self::Dimension { kind, unit } => {
                write!(f, "quantity kind has dimension {kind}, the unit has {unit}")
            }
            Self::Mismatch { left, right } => write!(
                f,
                "{left} and {right} are not interchangeable: no shared equivalence class"
            ),
        }
    }
}

impl KindError {
    /// The public error class name shared with unitsml-ruby.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown(_) => "UnknownQuantityKindError",
            Self::Primary(_) => "PrimaryQuantityError",
            Self::Dimension { .. } => "DimensionMismatchError",
            Self::Mismatch { .. } => "QuantityKindMismatchError",
        }
    }
}

/// One UnitsDB quantity used as a quantity kind.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct QuantityKind {
    record: &'static Value,
}

impl QuantityKind {
    /// Resolve a kind by quantity identifier (any identifier) or by name.
    pub fn resolve(key: &str) -> Result<Self, KindError> {
        if key.trim().is_empty() {
            return Err(KindError::Unknown(key.to_owned()));
        }
        find(Lookup::QuantityId, key)
            .or_else(|| {
                records(Collection::Quantities).iter().find(|record| {
                    record["identifiers"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id["id"] == key))
                })
            })
            .or_else(|| {
                records(Collection::Quantities).iter().find(|record| {
                    record["names"]
                        .as_array()
                        .is_some_and(|names| names.iter().any(|n| n["value"] == key))
                })
            })
            .map(|record| Self { record })
            .ok_or_else(|| KindError::Unknown(key.to_owned()))
    }

    /// The primary quantity of a UnitsDB unit (by unit identifier).
    /// `Ok(None)` when the unit has no quantity references.
    pub fn primary_for_unit(unit_id: &str) -> Result<Option<Self>, KindError> {
        let unit = find(Lookup::UnitId, unit_id)
            .ok_or_else(|| KindError::Primary(format!("unknown unit {unit_id:?}")))?;
        Self::primary_among(
            unit_id,
            unit["quantity_references"]
                .as_array()
                .map_or(&[][..], Vec::as_slice),
        )
    }

    /// The unit's UnitsDB `quantity_references`, in data order.
    pub fn unit_references(unit_id: &str) -> Option<&'static [Value]> {
        find(Lookup::UnitId, unit_id)?["quantity_references"]
            .as_array()
            .map(Vec::as_slice)
    }

    /// Primary selection over a reference list; independent of list order.
    pub fn primary_among(unit_id: &str, refs: &[Value]) -> Result<Option<Self>, KindError> {
        if refs.is_empty() {
            return Ok(None);
        }
        let flagged: Vec<_> = refs.iter().filter(|r| r["primary"] == true).collect();
        let chosen = match flagged.len() {
            1 => flagged[0],
            // A sole reference with NO primary flag is the unit's primary
            // quantity: UnitsDB marks primaries only where a unit has several
            // references (schema 2.2.0, unitsdb 6f3d156). An explicit
            // `primary: false` is never promoted (unitsml-ruby c4821ee).
            0 if refs.len() == 1 && refs[0].get("primary").is_none() => &refs[0],
            0 => {
                return Err(KindError::Primary(format!(
                    "unit {unit_id:?} has {} quantity reference(s) and none is marked primary",
                    refs.len()
                )))
            }
            n => {
                return Err(KindError::Primary(format!(
                    "unit {unit_id:?} marks {n} quantity references primary"
                )))
            }
        };
        let id = chosen["id"]
            .as_str()
            .ok_or_else(|| KindError::Primary(format!("unit {unit_id:?} reference has no id")))?;
        Self::resolve(id).map(Some)
    }

    /// The NIST identifier (or first identifier) of the quantity.
    pub fn id(&self) -> &'static str {
        let ids = self.record["identifiers"].as_array();
        ids.and_then(|ids| ids.iter().find(|i| i["type"] == "nist"))
            .or_else(|| ids.and_then(|ids| ids.first()))
            .and_then(|i| i["id"].as_str())
            .or_else(|| self.record["id"].as_str())
            .unwrap_or_default()
    }

    pub fn equivalence_class(&self) -> Option<&'static str> {
        self.record["equivalence_class"].as_str()
    }

    pub fn dimension_id(&self) -> Option<&'static str> {
        self.record
            .pointer("/dimension_reference/id")
            .and_then(Value::as_str)
    }

    /// Same quantity, or a shared non-null equivalence class.
    pub fn compatible(&self, other: &Self) -> bool {
        self.id() == other.id()
            || self
                .equivalence_class()
                .is_some_and(|class| Some(class) == other.equivalence_class())
    }

    /// Refuse a kind whose dimension differs from the unit's dimension;
    /// either side unknown is accepted, as in unitsml-ruby.
    pub fn check_dimension(&self, unit_dimension: Option<&str>) -> Result<(), KindError> {
        match (self.dimension_id(), unit_dimension) {
            (Some(kind), Some(unit)) if kind != unit => Err(KindError::Dimension {
                kind: kind.to_owned(),
                unit: unit.to_owned(),
            }),
            _ => Ok(()),
        }
    }
}

/// Kind of `left + right` / `left - right`: refused unless compatible;
/// an untyped side never refuses; the result keeps `left`, else `right`.
pub fn combine_kinds(
    left: Option<&QuantityKind>,
    right: Option<&QuantityKind>,
) -> Result<Option<QuantityKind>, KindError> {
    if let (Some(l), Some(r)) = (left, right) {
        if !l.compatible(r) {
            return Err(KindError::Mismatch {
                left: l.id().to_owned(),
                right: r.id().to_owned(),
            });
        }
    }
    Ok(left.or(right).cloned())
}
