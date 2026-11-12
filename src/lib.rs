//! Exact quantity operations over structural unit operands and the UnitsDB register.
mod asciiunits;
mod ffi;
mod kind;
mod register;
mod wire;

pub use asciiunits::terms_from_asciiunits;
pub use kind::{combine_kinds, KindError, QuantityKind};

pub use num_rational::BigRational as Rational;
pub use register::{Register, Registration};
pub use wire::{decode_register, evaluate_json, WireError};

use num_bigint::BigInt;
use num_traits::{One, Zero};
use std::{cmp::Ordering, collections::BTreeMap, fmt};

pub type Dimensions = BTreeMap<String, Rational>;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Scale {
    Ratio,
    Interval,
    Difference,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Error {
    UnsupportedOperand,
    UnknownUnit(String),
    UnknownPrefix(String),
    MissingConversion(String),
    InvalidRegister(String),
    InconsistentDimensions(String),
    UnsupportedScale(String),
    ScaleOperation {
        scale: Scale,
        operation: &'static str,
    },
    DimensionMismatch,
    AffineComposition,
    InexactPower,
    InvalidNumber(String),
    ResourceLimit,
    DivisionByZero,
}

impl Error {
    pub fn result_class(&self) -> &'static str {
        if *self == Self::DivisionByZero {
            "indeterminate"
        } else {
            "error"
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

/// Parse an exact decimal, integer, or numerator/denominator ratio.
pub fn rational(text: &str) -> Result<Rational, Error> {
    if text.len() > 8192 {
        return Err(Error::ResourceLimit);
    }
    let invalid = || Error::InvalidNumber(text.to_owned());
    if let Some((numerator, denominator)) = text.split_once('/') {
        let numerator = integer(numerator).ok_or_else(invalid)?;
        let denominator = integer(denominator).ok_or_else(invalid)?;
        if denominator.is_zero() {
            return Err(invalid());
        }
        return Ok(Rational::new(numerator, denominator));
    }
    let (decimal, exponent) = match text.split_once(['e', 'E']) {
        Some((decimal, exponent)) => (decimal, exponent.parse::<i32>().map_err(|_| invalid())?),
        None => (text, 0),
    };
    let unsigned = decimal.strip_prefix(['+', '-']).unwrap_or(decimal);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let digits = format!("{whole}{fraction}");
    let mut numerator = integer(&digits).ok_or_else(invalid)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    if decimal.starts_with('-') {
        numerator = -numerator;
    }
    let shift = i64::from(exponent) - fraction.len() as i64;
    if shift.unsigned_abs() > 8192 {
        return Err(Error::ResourceLimit);
    }
    let power = BigInt::from(10).pow(shift.unsigned_abs() as u32);
    Ok(if shift < 0 {
        Rational::new(numerator, power)
    } else {
        Rational::from_integer(numerator * power)
    })
}

fn integer(text: &str) -> Option<BigInt> {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// An already parsed leaf. IDs and unambiguous UnitsDB lookup spellings are accepted.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UnitTerm {
    pub unit: String,
    pub prefix: Option<String>,
    pub power: Rational,
}

impl UnitTerm {
    pub fn new(unit: impl Into<String>) -> Self {
        Self {
            unit: unit.into(),
            prefix: None,
            power: Rational::one(),
        }
    }
}

/// Unit identity as registered by UnitsDB, independent of its conversion data.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IdentifiedUnit {
    terms: Vec<UnitTerm>,
}

impl IdentifiedUnit {
    pub fn terms(&self) -> &[UnitTerm] {
        &self.terms
    }
}

/// A magnitude with known unit identities. Resolve conversion metadata before
/// doing dimensional or scale-aware arithmetic.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IdentifiedQuantity {
    magnitude: Rational,
    unit: IdentifiedUnit,
}

impl IdentifiedQuantity {
    pub fn new(magnitude: Rational, unit: IdentifiedUnit) -> Self {
        Self { magnitude, unit }
    }

    pub fn magnitude(&self) -> &Rational {
        &self.magnitude
    }

    pub fn unit(&self) -> &IdentifiedUnit {
        &self.unit
    }

    pub fn with_conversions(&self, register: &Register) -> Result<Quantity, Error> {
        Ok(Quantity::new(
            self.magnitude.clone(),
            register.resolve_identified(&self.unit)?,
        ))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Unit {
    terms: Vec<UnitTerm>,
    dimensions: Dimensions,
    factor: Rational,
    offset: Rational,
    scale: Scale,
}

impl Unit {
    pub fn terms(&self) -> &[UnitTerm] {
        &self.terms
    }

    pub fn dimensions(&self) -> &Dimensions {
        &self.dimensions
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    /// Match an exact nonempty dimension vector against the UnitsDB catalog.
    pub fn dimension_id(&self) -> Option<&'static str> {
        register::dimension_id(&self.dimensions)
    }

    pub fn factor(&self) -> &Rational {
        &self.factor
    }

    pub fn offset(&self) -> &Rational {
        &self.offset
    }

    fn identity() -> Self {
        Self {
            terms: Vec::new(),
            dimensions: Dimensions::new(),
            factor: Rational::one(),
            offset: Rational::zero(),
            scale: Scale::Ratio,
        }
    }

    fn combine(&self, other: &Self, divide: bool) -> Self {
        let mut dimensions = self.dimensions.clone();
        for (name, power) in &other.dimensions {
            *dimensions.entry(name.clone()).or_default() +=
                if divide { -power } else { power.clone() };
        }
        dimensions.retain(|_, power| !power.is_zero());
        let factor = if divide {
            &self.factor / &other.factor
        } else {
            &self.factor * &other.factor
        };
        let mut powers = BTreeMap::<_, Rational>::new();
        for (terms, inverse) in [(&self.terms, false), (&other.terms, divide)] {
            for term in terms {
                *powers
                    .entry((term.unit.clone(), term.prefix.clone()))
                    .or_default() += if inverse {
                    -&term.power
                } else {
                    term.power.clone()
                };
            }
        }
        let terms = powers
            .into_iter()
            .filter_map(|((unit, prefix), power)| {
                (!power.is_zero()).then_some(UnitTerm {
                    unit,
                    prefix,
                    power,
                })
            })
            .collect();
        Self {
            terms,
            dimensions,
            factor,
            ..Self::identity()
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Quantity {
    magnitude: Rational,
    unit: Unit,
    scale: Scale,
}

impl Quantity {
    pub fn new(magnitude: Rational, unit: Unit) -> Self {
        let scale = unit.scale;
        Self {
            magnitude,
            unit,
            scale,
        }
    }

    pub fn scalar(magnitude: Rational) -> Self {
        Self::new(magnitude, Unit::identity())
    }

    pub fn difference(magnitude: Rational, unit: Unit) -> Self {
        Self {
            magnitude,
            unit,
            scale: Scale::Difference,
        }
    }

    pub fn magnitude(&self) -> &Rational {
        &self.magnitude
    }

    pub fn unit(&self) -> &Unit {
        &self.unit
    }

    pub fn scale(&self) -> Scale {
        self.scale
    }

    pub fn canonical_magnitude(&self) -> Rational {
        let value = &self.magnitude * &self.unit.factor;
        if self.scale == Scale::Difference {
            value
        } else {
            value + &self.unit.offset
        }
    }

    pub fn convert(&self, unit: &Unit) -> Result<Self, Error> {
        self.check_dimensions(unit)?;
        let mut value = self.canonical_magnitude();
        if self.scale != Scale::Difference {
            value -= &unit.offset;
        }
        let magnitude = value / &unit.factor;
        Ok(if self.scale == Scale::Difference {
            Self::difference(magnitude, unit.clone())
        } else {
            Self::new(magnitude, unit.clone())
        })
    }

    pub fn add(&self, rhs: &Self) -> Result<Self, Error> {
        self.sum(rhs, false)
    }

    pub fn subtract(&self, rhs: &Self) -> Result<Self, Error> {
        self.sum(rhs, true)
    }

    fn sum(&self, rhs: &Self, subtract: bool) -> Result<Self, Error> {
        self.check_dimensions(&rhs.unit)?;
        let (absolute, difference) = (
            self.scale != Scale::Difference,
            rhs.scale == Scale::Difference,
        );
        if !absolute && !difference && !subtract {
            return rhs.add(self);
        }
        let aligned = rhs.convert(&self.unit)?.magnitude;
        let magnitude = if subtract {
            &self.magnitude - aligned
        } else {
            &self.magnitude + aligned
        };
        Ok(if (!absolute && difference) || (subtract && !difference) {
            Self::difference(magnitude, self.unit.clone())
        } else {
            Self::new(magnitude, self.unit.clone())
        })
    }

    pub fn compare(&self, rhs: &Self) -> Result<Ordering, Error> {
        self.check_dimensions(&rhs.unit)?;
        if (self.scale == Scale::Difference) != (rhs.scale == Scale::Difference) {
            return Err(Error::ScaleOperation {
                scale: Scale::Difference,
                operation: "comparison with absolute",
            });
        }
        Ok(self.canonical_magnitude().cmp(&rhs.canonical_magnitude()))
    }

    pub fn multiply(&self, rhs: &Self) -> Result<Self, Error> {
        self.product(rhs, false)
    }

    pub fn divide(&self, rhs: &Self) -> Result<Self, Error> {
        self.product(rhs, true)
    }

    pub fn ratio(&self, rhs: &Self) -> Result<Rational, Error> {
        self.check_dimensions(&rhs.unit)?;
        self.require_ratio("ratio")?;
        rhs.require_ratio("ratio")?;
        Ok(self.divide(rhs)?.canonical_magnitude())
    }

    fn product(&self, rhs: &Self, divide: bool) -> Result<Self, Error> {
        let operation = if divide { "division" } else { "multiplication" };
        self.require_ratio(operation)?;
        rhs.require_ratio(operation)?;
        let magnitude = if divide {
            if rhs.magnitude.is_zero() {
                return Err(Error::DivisionByZero);
            }
            &self.magnitude / &rhs.magnitude
        } else {
            &self.magnitude * &rhs.magnitude
        };
        Ok(Self::new(magnitude, self.unit.combine(&rhs.unit, divide)))
    }

    pub fn scale_by(&self, factor: &Rational) -> Result<Self, Error> {
        self.require_ratio("scaling")?;
        Ok(Self {
            magnitude: &self.magnitude * factor,
            ..self.clone()
        })
    }

    pub fn divide_by(&self, divisor: &Rational) -> Result<Self, Error> {
        if divisor.is_zero() {
            return Err(Error::DivisionByZero);
        }
        self.scale_by(&divisor.recip())
    }

    fn check_dimensions(&self, rhs: &Unit) -> Result<(), Error> {
        if self.unit.dimensions != rhs.dimensions {
            return Err(Error::DimensionMismatch);
        }
        Ok(())
    }

    fn require_ratio(&self, operation: &'static str) -> Result<(), Error> {
        if self.scale == Scale::Interval {
            return Err(Error::ScaleOperation {
                scale: self.scale,
                operation,
            });
        }
        Ok(())
    }
}
