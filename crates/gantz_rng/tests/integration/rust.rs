//! Tests for the Rust core. The golden values were computed independently
//! from the algorithm and encoding in the crate docs. They pin the outputs
//! across versions and platforms.

use gantz_rng::*;
use num_bigint::BigInt;
use num_rational::{BigRational, Rational64};

const S: Seed = Seed(7);

fn ratio(numer: i64, denom: i64) -> Rational64 {
    Rational64::new(numer, denom)
}

fn big(n: i128) -> BigInt {
    BigInt::from(n)
}

fn big_ratio(numer: i128, denom: i128) -> BigRational {
    BigRational::new(big(numer), big(denom))
}

// `mix` is the SplitMix64 output fn. For seed 0, SplitMix64 outputs
// `mix(k * gamma)` in turn.
#[test]
fn mix_matches_the_splitmix64_vector() {
    const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
    let expected = [
        0xe220a8397b1dcdaf,
        0x6e789e6aa1b965f4,
        0x06c45d188009454f,
        0xf88bb8a8724c81ec,
        0x1b39896a51a8749b,
    ];
    for (k, expected) in expected.into_iter().enumerate() {
        assert_eq!(mix((k as u64).wrapping_mul(GAMMA)), expected);
    }
}

// Folds follow the encoding in the crate docs.
#[test]
fn fold_goldens() {
    assert_eq!(fold_int(S, 3), Seed(0x4a298d1b0e808629));
    assert_eq!(fold_ratio(S, ratio(1, 2)), Seed(0x0f3c3213eb3460ee));
    assert_eq!(fold_int(S, -1), Seed(0x0261248d42ebeb46));
    assert_eq!(fold_int(S, 1 << 63), Seed(0xd78e3bf4662c22dc));
    assert_eq!(fold_str(S, "ab"), Seed(0xeb7727ad4e188e73));
    assert_eq!(fold_sequence(Seed(0), 0), Seed(0x17e757f16cfb68cf));
    // The list `(1 (2))`.
    let list = fold_sequence(S, 2);
    let list = fold_int(list, 1);
    let list = fold_sequence(list, 1);
    let list = fold_int(list, 2);
    assert_eq!(list, Seed(0x5d78f8673eab5eef));
    // The pair `(1/4 . 1/2)`.
    let pair = fold_ratio(fold_pair(S), ratio(1, 4));
    let pair = fold_ratio(pair, ratio(1, 2));
    assert_eq!(pair, Seed(0xb85bd15d1d9748b1));
}

// Draws follow the formulas in their docs.
#[test]
fn draw_goldens() {
    let seed = Seed(42);
    assert_eq!(uniform(seed), 6679422623415661.0 / (1u64 << 53) as f64);
    assert_eq!(integer(seed, 0, 10), 7);
    assert_eq!(integer(seed, -5, 5), 2);
    assert_eq!(choose_index(seed, 3), Some(2));
    assert_eq!(
        shuffle(seed, (0..8).collect()),
        vec![7, 2, 5, 6, 1, 3, 4, 0]
    );
}

// A number folds in by exact value, whatever its representation.
#[test]
fn numbers_fold_by_value() {
    let three = fold_int(S, 3);
    assert_eq!(fold_ratio(S, ratio(6, 2)), three);
    assert_eq!(fold_float(S, 3.0), three);
    assert_eq!(fold_big_int(S, &big(3)), three);
    assert_eq!(fold_big_ratio(S, &big_ratio(6, 2)), three);

    let half = fold_ratio(S, ratio(1, 2));
    assert_eq!(fold_float(S, 0.5), half);
    assert_eq!(fold_big_ratio(S, &big_ratio(2, 4)), half);
    assert_eq!(fold_ratio(S, ratio(-1, -2)), half);
    assert_eq!(fold_float(S, -0.5), fold_ratio(S, ratio(1, -2)));

    assert_eq!(fold_float(S, -0.0), fold_int(S, 0));

    // Word-count boundaries agree between the i128 and big int paths.
    for n in [
        i128::from(i64::MIN),
        i128::from(i64::MAX),
        1 << 63,
        -(1 << 63) - 1,
        1 << 100,
        i128::MIN,
        i128::MAX,
    ] {
        assert_eq!(fold_int(S, n), fold_big_int(S, &big(n)), "{n}");
    }

    // The smallest subnormal float is 1 / 2^1074.
    let tiny = BigRational::new(big(1), BigInt::from(2).pow(1074));
    assert_eq!(fold_float(S, f64::from_bits(1)), fold_big_ratio(S, &tiny));
}

// NaN, +inf and -inf have their own encodings. All NaNs are equal.
#[test]
fn non_finite_floats() {
    let nan = fold_float(S, f64::NAN);
    assert_eq!(fold_float(S, f64::from_bits(f64::NAN.to_bits() | 1)), nan);
    let pos = fold_float(S, f64::INFINITY);
    let neg = fold_float(S, f64::NEG_INFINITY);
    assert_ne!(nan, pos);
    assert_ne!(nan, neg);
    assert_ne!(pos, neg);
}

// Values of different types never share an encoding.
#[test]
fn types_are_distinct() {
    let seeds = [
        fold_void(S),
        fold_bool(S, true),
        fold_char(S, '1'),
        fold_int(S, 1),
        fold_str(S, "1"),
        fold_symbol(S, "1"),
        fold_sequence(S, 1),
        fold_pair(S),
        fold_opaque(S),
        fold_str(S, ""),
        fold_sequence(S, 0),
    ];
    for (i, a) in seeds.iter().enumerate() {
        for b in &seeds[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

// Integers stay in their range. An empty range gives `lo`.
#[test]
fn integer_bounds() {
    assert_eq!(integer(S, 5, 5), 5);
    assert_eq!(integer(S, 5, -5), 5);
    let mut seen = [false; 6];
    for seed in split(S, 1000) {
        let n = integer(seed, -3, 3);
        assert!((-3..3).contains(&n));
        seen[(n + 3) as usize] = true;
        let full = integer(seed, i64::MIN, i64::MAX);
        assert!(full < i64::MAX);
    }
    assert!(seen.iter().all(|&s| s), "every value is drawn");
}

// Uniform floats stay in `[0, 1)`, with a mean near 1/2.
#[test]
fn uniform_range() {
    let n = 10_000;
    let sum: f64 = split(S, n)
        .map(uniform)
        .inspect(|f| assert!((0.0..1.0).contains(f)))
        .sum();
    let mean = sum / n as f64;
    assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
}

// A probability of 0 or less, or NaN, is never true. 1 or more always is.
#[test]
fn bernoulli_extremes() {
    for seed in split(S, 100) {
        assert!(!bernoulli(seed, 0.0));
        assert!(!bernoulli(seed, -1.0));
        assert!(!bernoulli(seed, f64::NAN));
        assert!(bernoulli(seed, 1.0));
        assert!(bernoulli(seed, 2.0));
    }
}

// Split seed `i` is the seed with `i` folded in.
#[test]
fn split_folds_in_indices() {
    let seeds: Vec<Seed> = split(S, 4).collect();
    let expected: Vec<Seed> = (0..4).map(|i| fold_int(S, i)).collect();
    assert_eq!(seeds, expected);
}

// A shuffle is a permutation, and different seeds give different orders.
#[test]
fn shuffle_permutes() {
    let xs: Vec<u32> = (0..10).collect();
    let a = shuffle(Seed(1), xs.clone());
    let b = shuffle(Seed(2), xs.clone());
    assert_ne!(a, b);
    for mut ys in [a, b] {
        ys.sort();
        assert_eq!(ys, xs);
    }
    assert_eq!(shuffle(S, Vec::<u32>::new()), Vec::<u32>::new());
    assert_eq!(shuffle(S, vec![1]), vec![1]);
}

// No index for an empty sequence.
#[test]
fn choose_index_of_empty() {
    assert_eq!(choose_index(S, 0), None);
    assert_eq!(choose_index(S, 1), Some(0));
}
