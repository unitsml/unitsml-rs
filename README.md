# UnitsML Rust

Exact quantity algebra over the pinned UnitsDB v2.2.0 register. Construct operands
from resolved unit leaves with signed rational powers or pass the structural model
from the AsciiUnits Rust parser to `terms_from_asciiunits`. The crate has no
independent text parser.

```rust
use unitsml_rs::{rational, Quantity, Register, UnitTerm};

let db = Register::default();
let celsius = db.resolve(&[UnitTerm::new("degC")])?;
let kelvin = db.resolve(&[UnitTerm::new("K")])?;
let room = Quantity::new(rational("20")?, celsius);
let converted = room.convert(&kelvin)?;
assert_eq!(converted.magnitude(), &rational("293.15")?);
# Ok::<(), unitsml_rs::Error>(())
```

Use `Register::identify` and `IdentifiedQuantity::new` to construct a quantity
from known UnitsDB identities even when conversion or scale data is unavailable
or inconsistent. `IdentifiedQuantity::with_conversions` checks those data
before producing a quantity suitable for arithmetic.

Magnitudes and dimension exponents are arbitrary-precision rationals. Composite
units retain the exact canonical factor and cancel zero dimension exponents.
`dimension_id` returns a UnitsDB ID when the nonempty integer dimension vector is
registered. Compare quantities with `compare`, which aligns canonical values and
checks dimensions. Structural Rust equality compares the represented quantity.

Absolute intervals cannot multiply, divide, or scale. Subtraction of absolute
values gives a difference. A difference converts without either endpoint offset.
Adding a difference to an absolute value preserves the absolute endpoint. For
two absolute values, addition uses the left operand's coordinates after aligning
the right, matching the admitted reference convention. Divide-by-zero has result
class `indeterminate`; other typed refusals have class `error`; `Ok` is a value.

`Register::insert` accepts caller affine coefficients and an exact dimension
vector. The vector must match the pinned structure and any coefficients already
published by UnitsDB must agree within the reference relative tolerance of
10⁻⁹ (exact at zero). Register instances are independent. No exchange-rate,
logarithmic or discrete operations, XML conversion-definition interpreter,
uncertainty, or GUM evaluator are provided. Unknown units and conversions without
a usable structural path are refused. If decomposition disagrees with the
register's primary quantity dimension, resolution returns
`Error::InconsistentDimensions` naming the conflicting record. Affine composition and irrational factor
powers are refused. A product whose individually irrational factors would cancel
is outside this resolved-leaf profile.

Local package installation requires the separately packaged `unitsdb-rs 0.1.0`
artifact. Neither crate needs a runtime network lookup.

## C and JSON entry points

The native library exports ABI version 1 through `include/unitsml.h`. Pass a
length-delimited UTF-8 JSON request to `unitsml_evaluate_json_v1` and free its
returned buffer with `unitsml_free_json_v1`. Rust callers can also use
`evaluate_json`.

```json
{"op":"convert","left":{"value":"20","model":{"unitName":"degC"}},"right":{"terms":[{"unit":"K"}]}}
```

Operations are `identify`, `resolve`, `convert`, `add`, `subtract`, `multiply`, `divide`,
`compare`, `ratio`, `scale`, and `divide_by`. An operand has a string `value`,
optional boolean `difference`, and either `model` or `terms`. A term has a
string `unit` and optional string `prefix` and `power`. Omitting both unit forms
makes a scalar. `convert` needs only the target unit on the right. `scale` and
`divide_by` take a string `scalar`. Optional `register` entries supply
`unit`, `dimensions`, `factor`, and `offset` (default zero). Numeric fields
and register dimension exponents are exact strings.

Replies carry `abi_version` and `class`. An `identify` value contains exact `magnitude` and resolved `terms`. A conversion-ready quantity value contains `magnitude`,
`canonical_magnitude`, `dimension_id`, `dimensions`, `scale`, and resolved
`terms`. `compare` returns −1, 0, or 1 and `ratio` returns a rational string.
Refusals carry an `error` object with a stable `code` and a diagnostic `detail`.
