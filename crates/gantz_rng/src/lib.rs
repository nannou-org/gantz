//! Seeded, stateless random numbers.
//!
//! A [`Seed`] is 64 bits. Every value this crate produces is a pure
//! function of a seed and the other arguments. A seed gives the same values
//! on every run, on every platform and in every version. There is no
//! generator state to pass through a program.
//!
//! - Derive new seeds with the `fold_*` fns, which fold a key value into a
//!   seed, or with [`split`].
//! - Draw values with [`uniform`], [`integer`], [`bernoulli`],
//!   [`choose_index`] and [`shuffle`].
//!
//! Every draw uses up its seed. Two draws from the same seed give
//! correlated results. For example, [`uniform`] and [`integer`] agree, and
//! [`shuffle`] agrees with [`split`]. Fold a different key into the seed
//! for each draw.
//!
//! The [`steel`] module provides these fns to Steel as the `gantz/rng`
//! module.
//!
//! # Algorithm
//!
//! [`mix`] is the SplitMix64 output function. To fold a key into a seed,
//! start from the seed and absorb each 64-bit word `w` of the key's
//! encoding as `h = mix(h ^ w)`.
//!
//! # Key encoding
//!
//! A key encodes as a sequence of 64-bit words that starts with a tag. The
//! encoding depends only on the key's value, never on how it is stored.
//!
//! | Key | Words |
//! |---|---|
//! | void | `1` |
//! | boolean | `2`, then 0 or 1 |
//! | char | `3`, then the Unicode scalar value |
//! | number | `4`, then the numerator and the denominator |
//! | NaN | `5` |
//! | +inf | `6` |
//! | -inf | `7` |
//! | string | `8`, then the byte length and the bytes |
//! | symbol | `9`, then the byte length and the bytes |
//! | sequence | `10`, then the length and each element |
//! | pair | `11`, then the first element and the second element |
//! | opaque | `12` |
//!
//! A number is its exact value as a fraction in lowest terms with a
//! positive denominator. So `3`, `3.0` and `6/2` encode the same, as do
//! `0.5` and `1/2`, and `-0.0` encodes as `0`. The numerator and the
//! denominator each encode as a word count, then the minimal two's
//! complement little-endian 64-bit words of the value. String and symbol
//! bytes pack into little-endian words, and zero bytes pad the last word.

use num_bigint::{BigInt, Sign};
use num_rational::{BigRational, Rational64};

pub mod steel;

pub use steel::{MODULE, modules};

/// The base `.gantz` source. Named graphs that wrap the `gantz/rng` fns as
/// thin expr nodes.
pub const BASE_BYTES: &[u8] = include_bytes!("../base.gantz");

/// A 64-bit seed. See the [crate docs](crate).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Seed(pub u64);

const VOID: u64 = 1;
const BOOL: u64 = 2;
const CHAR: u64 = 3;
const NUMBER: u64 = 4;
const NAN: u64 = 5;
const POS_INF: u64 = 6;
const NEG_INF: u64 = 7;
const STRING: u64 = 8;
const SYMBOL: u64 = 9;
const SEQUENCE: u64 = 10;
const PAIR: u64 = 11;
const OPAQUE: u64 = 12;

/// The SplitMix64 output for the state `x`.
///
/// This is the Stafford variant 13 finalizer applied to `x` plus the
/// golden gamma. It is a bijection, and `mix(0)` is the first output of
/// SplitMix64 for seed 0.
pub fn mix(x: u64) -> u64 {
    let z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fold void into the seed.
pub fn fold_void(seed: Seed) -> Seed {
    absorb(seed, VOID)
}

/// Fold a boolean into the seed.
pub fn fold_bool(seed: Seed, b: bool) -> Seed {
    absorb_all(seed, [BOOL, u64::from(b)])
}

/// Fold a char into the seed.
pub fn fold_char(seed: Seed, c: char) -> Seed {
    absorb_all(seed, [CHAR, u64::from(u32::from(c))])
}

/// Fold an integer into the seed.
pub fn fold_int(seed: Seed, n: i128) -> Seed {
    let (numer, len) = i128_words(n);
    fold_number(seed, &numer[..len], &[1])
}

/// Fold a big integer into the seed.
pub fn fold_big_int(seed: Seed, n: &BigInt) -> Seed {
    fold_number(seed, &big_int_words(n), &[1])
}

/// Fold a fraction into the seed.
///
/// `r` needs a non-zero denominator, as [`Rational64::new`] ensures. It
/// need not be in lowest terms.
pub fn fold_ratio(seed: Seed, r: Rational64) -> Seed {
    let r = r.reduced();
    let (numer, numer_len) = i128_words(i128::from(*r.numer()));
    let (denom, denom_len) = i128_words(i128::from(*r.denom()));
    fold_number(seed, &numer[..numer_len], &denom[..denom_len])
}

/// Fold a big fraction into the seed.
///
/// `r` needs a non-zero denominator, as [`BigRational::new`] ensures. It
/// need not be in lowest terms.
pub fn fold_big_ratio(seed: Seed, r: &BigRational) -> Seed {
    let r = r.reduced();
    fold_number(seed, &big_int_words(r.numer()), &big_int_words(r.denom()))
}

/// Fold a float into the seed.
///
/// A finite float folds in as the exact fraction it represents. NaN, +inf
/// and -inf each have their own encoding.
pub fn fold_float(seed: Seed, f: f64) -> Seed {
    match BigRational::from_float(f) {
        Some(r) => fold_big_ratio(seed, &r),
        None if f.is_nan() => absorb(seed, NAN),
        None if f > 0.0 => absorb(seed, POS_INF),
        None => absorb(seed, NEG_INF),
    }
}

/// Fold a string into the seed.
pub fn fold_str(seed: Seed, s: &str) -> Seed {
    fold_text(seed, STRING, s)
}

/// Fold a symbol into the seed. Unlike [`fold_str`], the text is a symbol.
pub fn fold_symbol(seed: Seed, s: &str) -> Seed {
    fold_text(seed, SYMBOL, s)
}

/// Start to fold a sequence of `len` elements into the seed. Fold in each
/// element next, in order.
pub fn fold_sequence(seed: Seed, len: u64) -> Seed {
    absorb_all(seed, [SEQUENCE, len])
}

/// Start to fold a pair into the seed. Fold in its first element and then
/// its second element next.
pub fn fold_pair(seed: Seed) -> Seed {
    absorb(seed, PAIR)
}

/// Fold an opaque value into the seed. All opaque values fold in the same.
pub fn fold_opaque(seed: Seed) -> Seed {
    absorb(seed, OPAQUE)
}

/// The seeds `fold_int(seed, i)` for `i` in `0..n`.
pub fn split(seed: Seed, n: u64) -> impl Iterator<Item = Seed> {
    (0..n).map(move |i| fold_int(seed, i128::from(i)))
}

/// A float in `[0, 1)`, with 53 random bits.
pub fn uniform(seed: Seed) -> f64 {
    const SCALE: f64 = 1.0 / (1u64 << 53) as f64;
    (mix(seed.0) >> 11) as f64 * SCALE
}

/// An integer in `[lo, hi)`, or `lo` when `hi <= lo`.
///
/// The draw maps 64 random bits onto the range with a widening multiply.
/// Its bias is below `(hi - lo) / 2^64`.
pub fn integer(seed: Seed, lo: i64, hi: i64) -> i64 {
    if hi <= lo {
        return lo;
    }
    lo.wrapping_add_unsigned(below(seed, hi.abs_diff(lo)))
}

/// `true` with the probability `p`.
///
/// A `p` of 0 or less, or NaN, is never `true`. A `p` of 1 or more is
/// always `true`.
pub fn bernoulli(seed: Seed, p: f64) -> bool {
    uniform(seed) < p
}

/// An index into a sequence of `len` elements, or `None` when `len` is 0.
pub fn choose_index(seed: Seed, len: usize) -> Option<usize> {
    (len > 0).then(|| below(seed, len as u64) as usize)
}

/// The elements of `xs` in a uniformly random order.
///
/// This is Fisher-Yates. The swap at index `i` draws from
/// `fold_int(seed, i)`.
pub fn shuffle<T>(seed: Seed, mut xs: Vec<T>) -> Vec<T> {
    for i in (1..xs.len()).rev() {
        let j = below(fold_int(seed, i as i128), i as u64 + 1);
        xs.swap(i, j as usize);
    }
    xs
}

fn absorb(seed: Seed, word: u64) -> Seed {
    Seed(mix(seed.0 ^ word))
}

fn absorb_all(seed: Seed, words: impl IntoIterator<Item = u64>) -> Seed {
    words.into_iter().fold(seed, absorb)
}

/// A value in `[0, n)`, from a widening multiply of 64 random bits.
fn below(seed: Seed, n: u64) -> u64 {
    ((u128::from(mix(seed.0)) * u128::from(n)) >> 64) as u64
}

fn fold_number(seed: Seed, numer: &[u64], denom: &[u64]) -> Seed {
    let seed = absorb_all(seed, [NUMBER, numer.len() as u64]);
    let seed = absorb_all(seed, numer.iter().copied());
    let seed = absorb(seed, denom.len() as u64);
    absorb_all(seed, denom.iter().copied())
}

fn fold_text(seed: Seed, tag: u64, s: &str) -> Seed {
    let seed = absorb_all(seed, [tag, s.len() as u64]);
    absorb_all(seed, le_words(s.as_bytes(), 0))
}

/// The minimal two's complement little-endian words of `n`, at least one.
fn i128_words(n: i128) -> ([u64; 2], usize) {
    let lo = n as u64;
    let hi = (n >> 64) as u64;
    let len = if i128::from(lo as i64) == n { 1 } else { 2 };
    ([lo, hi], len)
}

/// The minimal two's complement little-endian words of `n`, at least one.
fn big_int_words(n: &BigInt) -> Vec<u64> {
    let fill = match n.sign() {
        Sign::Minus => 0xFF,
        Sign::NoSign | Sign::Plus => 0,
    };
    le_words(&n.to_signed_bytes_le(), fill).collect()
}

/// Pack bytes into little-endian words, padding the last word with `fill`.
fn le_words(bytes: &[u8], fill: u8) -> impl Iterator<Item = u64> + '_ {
    bytes.chunks(8).map(move |chunk| {
        let mut word = [fill; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        u64::from_le_bytes(word)
    })
}
