//! The data form of values, and the one bridge between it and Steel values.
//!
//! [`Datum`] and its serde codec come from `gantz_ca`. [`to_steel`] and
//! [`of_steel`] convert between a datum and a Steel value. A sequence is a
//! Steel list, and a map is a Steel hash map with symbol keys. A Steel vector
//! reads as a sequence and a string key reads as a name, so these two forms
//! become a list and a symbol key on a round trip.

pub use gantz_ca::datum::*;

use num_bigint::BigInt;
use num_rational::{BigRational, Rational32};
use num_traits::ToPrimitive;
use std::fmt;
use steel::SteelVal;
use steel::gc::Gc;
use steel::primitives::bytevectors;
use steel::rvals::SteelByteVector;

/// A Steel value with no data form, such as a function, and where it sits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotData {
    /// What the value is, such as "a function".
    pub what: String,
    /// The steps from the converted value down to the value.
    pub path: Vec<Step>,
}

/// A step into a sequence or a map. See [`NotData::path`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    Index(usize),
    Key(String),
}

/// The display of a [`NotData`] path, such as ` at [2].label`.
struct At<'a>(&'a [Step]);

/// The state of a conversion from a Steel value.
struct Walk {
    /// The steps from the converted value down to the current value.
    path: Vec<Step>,
    /// The address of each mutable vector that holds the current value.
    outer: Vec<usize>,
    /// Whether a value with no data form becomes a placeholder, in place of
    /// a failure.
    lossy: bool,
}

impl Walk {
    fn new(lossy: bool) -> Self {
        Walk {
            path: vec![],
            outer: vec![],
            lossy,
        }
    }

    /// The result for the value at the path, which has no data form.
    fn not_data(&self, what: impl Into<String>) -> Result<Datum, String> {
        let what = what.into();
        match self.lossy {
            true => Ok(Datum::opaque(&what)),
            false => Err(what),
        }
    }
}

impl fmt::Display for NotData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.what, At(&self.path))
    }
}

impl std::error::Error for NotData {}

impl fmt::Display for At<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        f.write_str(" at ")?;
        for step in self.0 {
            match step {
                Step::Index(ix) => write!(f, "[{ix}]")?,
                Step::Key(key) => write!(f, ".{key}")?,
            }
        }
        Ok(())
    }
}

/// The Steel value of `datum`. See the module docs.
pub fn to_steel(datum: &Datum) -> SteelVal {
    match datum {
        Datum::Null => SteelVal::Void,
        Datum::Bool(b) => SteelVal::BoolV(*b),
        Datum::I64(n) => int(*n),
        Datum::U64(n) => int(*n),
        Datum::F64(x) => SteelVal::NumV(*x),
        Datum::Char(c) => SteelVal::CharV(*c),
        Datum::Str(s) => SteelVal::StringV(s.clone().into()),
        Datum::Bytes(b) => SteelVal::ByteVector(SteelByteVector::new(b.clone())),
        Datum::Seq(items) => SteelVal::ListV(items.iter().map(to_steel).collect()),
        Datum::Map(entries) => {
            let map = entries
                .iter()
                .map(|(k, v)| (SteelVal::SymbolV(k.clone().into()), to_steel(v)))
                .collect::<steel::HashMap<_, _>>();
            SteelVal::HashMapV(Gc::new(map).into())
        }
        Datum::Sym(name) => SteelVal::SymbolV(name.clone().into()),
        Datum::Ratio(r) => match (i32::try_from(r.numer()), i32::try_from(r.denom())) {
            (Ok(n), Ok(d)) => SteelVal::Rational(Rational32::new_raw(n, d)),
            _ => {
                let ratio = BigRational::new_raw(r.numer().into(), r.denom().into());
                SteelVal::BigRational(Gc::new(ratio))
            }
        },
    }
}

/// The datum of the Steel value `val`, or the value within it that has no data
/// form. See the module docs.
pub fn of_steel(val: &SteelVal) -> Result<Datum, NotData> {
    let mut walk = Walk::new(false);
    datum_of(val, &mut walk).map_err(|what| NotData {
        what,
        path: walk.path,
    })
}

/// The datum of the Steel value `val`, with a placeholder of
/// [`Datum::opaque`] in place of each value within it that has no data form.
/// The result is for display only. See [`of_steel`].
pub fn of_steel_lossy(val: &SteelVal) -> Datum {
    let mut walk = Walk::new(true);
    datum_of(val, &mut walk).unwrap_or_else(|what| Datum::opaque(&what))
}

/// The datum of `val`. On failure, the path of `walk` leads to the value that
/// has no data form.
fn datum_of(val: &SteelVal, walk: &mut Walk) -> Result<Datum, String> {
    let datum = match val {
        SteelVal::Void => Datum::Null,
        SteelVal::BoolV(b) => Datum::Bool(*b),
        SteelVal::CharV(c) => Datum::Char(*c),
        SteelVal::IntV(n) => Datum::I64(*n as i64),
        SteelVal::BigNum(n) => match (n.to_i64(), n.to_u64()) {
            (Some(n), _) => Datum::I64(n),
            (None, Some(n)) => Datum::U64(n),
            (None, None) => return walk.not_data("an integer beyond 64 bits"),
        },
        SteelVal::Rational(r) => match Datum::ratio((*r.numer()).into(), (*r.denom()).into()) {
            Some(ratio) => ratio,
            None => return walk.not_data("a ratio over zero"),
        },
        SteelVal::BigRational(r) => match r.numer().to_i64().zip(r.denom().to_i64()) {
            Some((n, d)) => match Datum::ratio(n, d) {
                Some(ratio) => ratio,
                None => return walk.not_data("a ratio over zero"),
            },
            None => return walk.not_data("a ratio beyond 64 bits"),
        },
        SteelVal::NumV(x) if x.is_finite() => Datum::F64(*x),
        SteelVal::NumV(_) => return walk.not_data("a non-finite number"),
        SteelVal::StringV(s) => Datum::Str(s.to_string()),
        SteelVal::SymbolV(s) => Datum::Sym(s.to_string()),
        SteelVal::ByteVector(b) => Datum::Bytes(bytes_of(b)),
        SteelVal::ListV(items) => Datum::Seq(seq_of(items.iter(), walk)?),
        SteelVal::VectorV(items) => Datum::Seq(seq_of(items.iter(), walk)?),
        SteelVal::MutableVector(items) => {
            let addr = items.as_ptr_usize();
            if walk.outer.contains(&addr) {
                return walk.not_data("a vector that holds itself");
            }
            walk.outer.push(addr);
            let items = seq_of(items.get().iter(), walk)?;
            walk.outer.pop();
            Datum::Seq(items)
        }
        SteelVal::HashMapV(map) => {
            let mut entries = Vec::with_capacity(map.len());
            for (k, v) in map.iter() {
                let key = match k {
                    SteelVal::SymbolV(s) | SteelVal::StringV(s) => s.to_string(),
                    _ => return walk.not_data(format!("a map key that is not a name: {k}")),
                };
                walk.path.push(Step::Key(key.clone()));
                let v = datum_of(v, walk)?;
                walk.path.pop();
                entries.push((key, v));
            }
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            if entries.windows(2).any(|w| w[0].0 == w[1].0) {
                return walk.not_data("a map with a string key and a symbol key of one name");
            }
            Datum::Map(entries)
        }
        other => return walk.not_data(what(other)),
    };
    Ok(datum)
}

/// The datums of `items`. See [`datum_of`].
fn seq_of<'a>(
    items: impl Iterator<Item = &'a SteelVal>,
    walk: &mut Walk,
) -> Result<Vec<Datum>, String> {
    let mut seq = vec![];
    for (ix, item) in items.enumerate() {
        walk.path.push(Step::Index(ix));
        seq.push(datum_of(item, walk)?);
        walk.path.pop();
    }
    Ok(seq)
}

/// The Steel integer `n`, a fixnum when it fits.
fn int<N: Copy + TryInto<isize> + Into<BigInt>>(n: N) -> SteelVal {
    match n.try_into() {
        Ok(n) => SteelVal::IntV(n),
        Err(_) => SteelVal::BigNum(Gc::new(n.into())),
    }
}

/// The bytes of a Steel byte vector.
fn bytes_of(bytes: &SteelByteVector) -> Vec<u8> {
    let Ok(SteelVal::ListV(items)) = bytevectors::bytes_to_list(bytes) else {
        return vec![];
    };
    let byte = |item: &SteelVal| match item {
        SteelVal::IntV(n) => u8::try_from(*n).ok(),
        _ => None,
    };
    items.iter().filter_map(byte).collect()
}

/// What a Steel value with no data form is.
fn what(val: &SteelVal) -> &'static str {
    match val {
        SteelVal::Closure(_)
        | SteelVal::FuncV(_)
        | SteelVal::MutFunc(_)
        | SteelVal::BoxedFunction(_)
        | SteelVal::BuiltIn(_)
        | SteelVal::ContinuationFunction(_) => "a function",
        SteelVal::Pair(_) => "an improper pair",
        SteelVal::HashSetV(_) => "a hash set",
        SteelVal::Complex(_) => "a complex number",
        SteelVal::Custom(_) | SteelVal::CustomStruct(_) => "a custom value",
        _ => "a runtime value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use steel::steel_vm::engine::Engine;

    /// The value of `src`. The value can refer to the heap of `vm`, so `vm`
    /// must outlive it.
    fn eval(vm: &mut Engine, src: &str) -> SteelVal {
        vm.run(src.to_string()).unwrap().pop().unwrap()
    }

    /// Each datum survives a trip through Steel.
    #[test]
    fn datums_round_trip_through_steel() {
        let datum = Datum::Seq(vec![
            Datum::Null,
            Datum::Bool(false),
            Datum::I64(i64::MIN),
            Datum::U64(u64::MAX),
            Datum::F64(-0.25),
            Datum::Char('λ'),
            Datum::Str("text".into()),
            Datum::Bytes(vec![1, 2, 255]),
            Datum::Map(vec![
                ("a".into(), Datum::Sym("bd".into())),
                ("b".into(), Datum::Seq(vec![])),
            ]),
            Datum::ratio(-1, 3).unwrap(),
            Datum::ratio(1, 1 << 40).unwrap(),
        ]);
        assert_eq!(of_steel(&to_steel(&datum)).unwrap(), datum);
    }

    /// Steel code reads a datum map by symbol keys, and its exact numbers
    /// stay exact.
    #[test]
    fn steel_reads_the_values_it_wrote() {
        let vm = &mut Engine::new_base();
        let state = eval(vm, "(hash 'value 1/3 'pending (list (list 0.5 #\\a)))");
        let datum = of_steel(&state).unwrap();
        let expected = Datum::Map(vec![
            (
                "pending".into(),
                Datum::Seq(vec![Datum::Seq(vec![Datum::F64(0.5), Datum::Char('a')])]),
            ),
            ("value".into(), Datum::ratio(1, 3).unwrap()),
        ]);
        assert_eq!(datum, expected);
        assert_eq!(
            of_steel(&eval(vm, "(vector 1 \"x\")")).unwrap(),
            Datum::Seq(vec![Datum::I64(1), Datum::Str("x".into())])
        );
    }

    /// A value with no data form fails with what it is and where it sits.
    #[test]
    fn a_function_is_not_data() {
        let vm = &mut Engine::new_base();
        let val = eval(vm, "(list 1 (hash 'f (lambda (x) x)))");
        let err = of_steel(&val).unwrap_err();
        assert_eq!(err.what, "a function");
        assert_eq!(err.path, vec![Step::Index(1), Step::Key("f".into())]);
        assert_eq!(err.to_string(), "a function at [1].f");
        assert!(of_steel(&eval(vm, "(cons 1 2)")).is_err());
        assert!(of_steel(&eval(vm, "(hash 1 2)")).is_err());
    }

    /// The lossy form puts a placeholder in place of each value with no data
    /// form, and keeps the rest.
    #[test]
    fn opaque_values_take_a_placeholder() {
        let vm = &mut Engine::new_base();
        let f = eval(vm, "(lambda (x) x)");
        let items = [SteelVal::IntV(1), f, SteelVal::NumV(f64::NAN)];
        let val = SteelVal::ListV(items.into_iter().collect());
        let datum = of_steel_lossy(&val);
        let expected = Datum::Seq(vec![
            Datum::I64(1),
            Datum::opaque("a function"),
            Datum::opaque("a non-finite number"),
        ]);
        assert_eq!(
            expected.as_seq().unwrap()[1].as_opaque(),
            Some("a function")
        );
        assert_eq!(datum, expected);
    }
}
