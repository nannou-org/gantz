//! The `gantz/rng` Steel module.
//!
//! The `#%gantz/rng` [`builtin`] module holds the Rust fns, and `rng.scm`
//! provides them as `gantz/rng`. Seeds are Steel integers. An output seed
//! is the seed's bits as an `i64`.

use crate::Seed;
use gantz_core::steel::{
    SteelErr, SteelVal,
    rerrs::ErrorKind,
    steel_vm::{builtin::BuiltInModule, register_fn::RegisterFn},
};
use gantz_core::vm::SteelModule;
use num_rational::Rational64;
use num_traits::ToPrimitive;

/// The `gantz/rng` Steel module.
///
/// Register via [`gantz_core::vm::new_engine`] or the app's steel-module
/// collection, then `(require "gantz/rng")` to use. All provided names
/// carry the `rng/` prefix.
pub const MODULE: SteelModule =
    SteelModule::new("gantz/rng", include_str!("rng.scm")).with_builtin(builtin);

/// 2^63 as a float.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
/// 2^64 as a float.
const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0;

/// The Steel modules provided by this crate.
pub fn modules() -> &'static [SteelModule] {
    const MODULES: &[SteelModule] = &[MODULE];
    MODULES
}

/// The `#%gantz/rng` builtin module that `rng.scm` provides.
pub fn builtin() -> BuiltInModule {
    let mut module = BuiltInModule::new("#%gantz/rng");
    module
        .register_fn("rng/fold-in", fold_in)
        .register_fn("rng/split", split)
        .register_fn("rng/uniform", uniform)
        .register_fn("rng/integer", integer)
        .register_fn("rng/bernoulli", bernoulli)
        .register_fn("rng/choose", choose)
        .register_fn("rng/shuffle", shuffle);
    module
}

/// The seed that a Steel value stands for.
///
/// An integer-valued number in `[-2^63, 2^64)` gives its low 64 bits. Any
/// other value folds into `Seed(0)` as a key.
pub fn seed_of(val: &SteelVal) -> Seed {
    exact_seed(val).unwrap_or_else(|| fold_value(Seed(0), val))
}

/// The Steel integer for a seed.
pub fn seed_val(seed: Seed) -> SteelVal {
    SteelVal::from(seed.0 as i64)
}

/// Fold a Steel value into the seed as a key. See the crate docs for the
/// encoding.
///
/// Lists and vectors fold in as sequences, and improper pairs as pairs.
/// Procedures, hash maps, structs, mutable vectors and other such values
/// fold in as opaque. The walk uses an explicit stack, so deep nesting
/// cannot overflow the call stack.
pub fn fold_value(seed: Seed, val: &SteelVal) -> Seed {
    let mut seed = seed;
    let mut stack = vec![val];
    while let Some(val) = stack.pop() {
        seed = match val {
            SteelVal::Void => crate::fold_void(seed),
            SteelVal::BoolV(b) => crate::fold_bool(seed, *b),
            SteelVal::CharV(c) => crate::fold_char(seed, *c),
            SteelVal::IntV(i) => crate::fold_int(seed, *i as i128),
            SteelVal::BigNum(n) => crate::fold_big_int(seed, n),
            SteelVal::Rational(r) => {
                let r = Rational64::new_raw(i64::from(*r.numer()), i64::from(*r.denom()));
                crate::fold_ratio(seed, r)
            }
            SteelVal::BigRational(r) => crate::fold_big_ratio(seed, r),
            SteelVal::NumV(f) => crate::fold_float(seed, *f),
            SteelVal::StringV(s) => crate::fold_str(seed, s),
            SteelVal::SymbolV(s) => crate::fold_symbol(seed, s),
            SteelVal::ListV(items) => {
                let start = stack.len();
                stack.extend(items.iter());
                stack[start..].reverse();
                crate::fold_sequence(seed, items.len() as u64)
            }
            SteelVal::VectorV(items) => {
                stack.extend(items.iter().rev());
                crate::fold_sequence(seed, items.len() as u64)
            }
            SteelVal::Pair(pair) => {
                stack.push(pair.cdr_ref());
                stack.push(pair.car_ref());
                crate::fold_pair(seed)
            }
            _ => crate::fold_opaque(seed),
        };
    }
    seed
}

/// The seed of an integer-valued number in `[-2^63, 2^64)`.
fn exact_seed(val: &SteelVal) -> Option<Seed> {
    match val {
        SteelVal::IntV(i) => Some(Seed(*i as i64 as u64)),
        SteelVal::BigNum(n) => i64::try_from(n.as_ref())
            .map(|i| i as u64)
            .or_else(|_| u64::try_from(n.as_ref()))
            .ok()
            .map(Seed),
        SteelVal::NumV(f) if f.fract() == 0.0 && (-TWO_POW_63..TWO_POW_64).contains(f) => {
            let bits = if *f < 0.0 {
                *f as i64 as u64
            } else {
                *f as u64
            };
            Some(Seed(bits))
        }
        _ => None,
    }
}

fn fold_in(seed: SteelVal, key: SteelVal) -> SteelVal {
    seed_val(fold_value(seed_of(&seed), &key))
}

fn split(seed: SteelVal, n: i64) -> SteelVal {
    let n = u64::try_from(n).unwrap_or(0);
    SteelVal::ListV(crate::split(seed_of(&seed), n).map(seed_val).collect())
}

fn uniform(seed: SteelVal) -> f64 {
    crate::uniform(seed_of(&seed))
}

fn integer(seed: SteelVal, lo: i64, hi: i64) -> i64 {
    crate::integer(seed_of(&seed), lo, hi)
}

fn bernoulli(seed: SteelVal, p: SteelVal) -> Result<bool, SteelErr> {
    let p = to_f64(&p).ok_or_else(|| type_err("rng/bernoulli", "a number probability", &p))?;
    Ok(crate::bernoulli(seed_of(&seed), p))
}

fn choose(seed: SteelVal, xs: SteelVal) -> Result<SteelVal, SteelErr> {
    let items = sequence(&xs).ok_or_else(|| type_err("rng/choose", "a list or vector", &xs))?;
    let ix = crate::choose_index(seed_of(&seed), items.len()).ok_or_else(|| {
        SteelErr::new(
            ErrorKind::ContractViolation,
            "rng/choose: cannot choose from an empty list".to_string(),
        )
    })?;
    Ok(items[ix].clone())
}

fn shuffle(seed: SteelVal, xs: SteelVal) -> Result<SteelVal, SteelErr> {
    let items = sequence(&xs).ok_or_else(|| type_err("rng/shuffle", "a list or vector", &xs))?;
    let shuffled = crate::shuffle(seed_of(&seed), items);
    Ok(SteelVal::ListV(shuffled.into_iter().collect()))
}

/// The elements of a list or vector. A mutable vector gives its current
/// elements.
fn sequence(val: &SteelVal) -> Option<Vec<SteelVal>> {
    match val {
        SteelVal::ListV(items) => Some(items.iter().cloned().collect()),
        SteelVal::VectorV(items) => Some(items.iter().cloned().collect()),
        SteelVal::MutableVector(items) => Some(items.get()),
        _ => None,
    }
}

fn to_f64(val: &SteelVal) -> Option<f64> {
    match val {
        SteelVal::NumV(f) => Some(*f),
        SteelVal::IntV(i) => Some(*i as f64),
        SteelVal::Rational(r) => Some(f64::from(*r.numer()) / f64::from(*r.denom())),
        SteelVal::BigNum(n) => n.to_f64(),
        SteelVal::BigRational(r) => r.to_f64(),
        _ => None,
    }
}

fn type_err(fn_name: &str, expected: &str, got: &SteelVal) -> SteelErr {
    SteelErr::new(
        ErrorKind::TypeMismatch,
        format!("{fn_name}: expected {expected}, got {got}"),
    )
}
