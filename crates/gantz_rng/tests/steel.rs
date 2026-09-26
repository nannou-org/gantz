//! Tests for the `gantz/rng` Steel module. Each fn must agree with the
//! Rust core, and any value must work as a seed.

use gantz_core::steel::{SteelVal, steel_vm::engine::Engine};
use gantz_rng::{Seed, steel::seed_val};
use num_bigint::BigInt;
use num_rational::Rational64;

fn engine() -> Engine {
    let mut vm = gantz_core::vm::new_engine(gantz_rng::modules());
    vm.run("(require \"gantz/rng\")".to_string())
        .expect("require gantz/rng");
    vm
}

/// Evaluate the snippet on a fresh engine, returning its last value.
fn eval(snippet: &str) -> SteelVal {
    let vals = engine()
        .run(snippet.to_string())
        .unwrap_or_else(|e| panic!("steel error: {e}\nin snippet: {snippet}"));
    vals.last().expect("a value").clone()
}

fn assert_same(a: &str, b: &str) {
    assert_eq!(eval(a), eval(b), "{a} and {b}");
}

// Each fn agrees with the core.
#[test]
fn fns_match_the_core() {
    let s = Seed(7);
    assert_eq!(
        eval("(rng/fold-in 7 3)"),
        seed_val(gantz_rng::fold_int(s, 3))
    );
    assert_eq!(
        eval("(rng/split 7 3)"),
        SteelVal::ListV(gantz_rng::split(s, 3).map(seed_val).collect())
    );
    assert_eq!(
        eval("(rng/uniform 7)"),
        SteelVal::NumV(gantz_rng::uniform(s))
    );
    assert_eq!(
        eval("(rng/integer 7 -5 5)"),
        SteelVal::from(gantz_rng::integer(s, -5, 5))
    );
    assert_eq!(
        eval("(rng/bernoulli 7 1/2)"),
        SteelVal::BoolV(gantz_rng::bernoulli(s, 0.5))
    );
    let ix = gantz_rng::choose_index(s, 3).unwrap();
    assert_eq!(
        eval("(rng/choose 7 '(a b c))"),
        eval(&format!("(list-ref '(a b c) {ix})"))
    );
    let order = gantz_rng::shuffle(s, vec![1, 2, 3, 4, 5]);
    let expected: Vec<String> = order.iter().map(ToString::to_string).collect();
    assert_same(
        "(rng/shuffle 7 '(1 2 3 4 5))",
        &format!("'({})", expected.join(" ")),
    );
}

// Keys fold in by value and by the walk order in the crate docs.
#[test]
fn keys_fold_by_value() {
    assert_same("(rng/fold-in 7 3)", "(rng/fold-in 7 3.0)");
    assert_same("(rng/fold-in 7 3)", "(rng/fold-in 7 6/2)");
    assert_same("(rng/fold-in 7 1/2)", "(rng/fold-in 7 0.5)");
    assert_same("(rng/fold-in 7 '(1 2))", "(rng/fold-in 7 #(1 2))");
    // A mutable vector can change or contain itself, so it is opaque.
    assert_eq!(
        eval("(rng/fold-in 7 (vector 1 2))"),
        seed_val(gantz_rng::fold_opaque(Seed(7)))
    );
    assert_eq!(
        eval("(rng/fold-in 7 1/2)"),
        seed_val(gantz_rng::fold_ratio(Seed(7), Rational64::new(1, 2)))
    );

    // The list `(1 (2))`.
    let list = gantz_rng::fold_sequence(Seed(7), 2);
    let list = gantz_rng::fold_int(list, 1);
    let list = gantz_rng::fold_sequence(list, 1);
    let list = gantz_rng::fold_int(list, 2);
    assert_eq!(eval("(rng/fold-in 7 '(1 (2)))"), seed_val(list));

    // The pair `(1/4 . 1/2)`, the shape of a pattern span.
    let pair = gantz_rng::fold_pair(Seed(7));
    let pair = gantz_rng::fold_ratio(pair, Rational64::new(1, 4));
    let pair = gantz_rng::fold_ratio(pair, Rational64::new(1, 2));
    assert_eq!(eval("(rng/fold-in 7 (cons 1/4 1/2))"), seed_val(pair));

    // Strings and symbols differ.
    assert_ne!(eval("(rng/fold-in 7 \"a\")"), eval("(rng/fold-in 7 'a)"));

    // Exact big numbers take the big paths.
    let big = BigInt::from(2).pow(70);
    assert_eq!(
        eval("(rng/fold-in 7 (expt 2 70))"),
        seed_val(gantz_rng::fold_big_int(Seed(7), &big))
    );
}

// Any value works as a seed. Integer-valued numbers in [-2^63, 2^64) are
// the seed modulo 2^64. Anything else folds into seed 0.
#[test]
fn any_value_is_a_seed() {
    assert_same("(rng/uniform 42)", "(rng/uniform 42.0)");
    assert_same("(rng/uniform -1)", "(rng/uniform 18446744073709551615)");
    assert_same("(rng/uniform -1)", "(rng/uniform -1.0)");
    assert_eq!(
        eval("(rng/uniform 18446744073709551616)"),
        SteelVal::NumV(gantz_rng::uniform(gantz_rng::fold_big_int(
            Seed(0),
            &BigInt::from(2).pow(64)
        )))
    );
    assert_eq!(
        eval("(rng/uniform 1/2)"),
        SteelVal::NumV(gantz_rng::uniform(gantz_rng::fold_ratio(
            Seed(0),
            Rational64::new(1, 2)
        )))
    );
    assert_eq!(
        eval("(rng/uniform '())"),
        SteelVal::NumV(gantz_rng::uniform(gantz_rng::fold_sequence(Seed(0), 0)))
    );
    assert_eq!(
        eval("(rng/uniform (lambda () 1))"),
        SteelVal::NumV(gantz_rng::uniform(gantz_rng::fold_opaque(Seed(0))))
    );
    for junk in ["void", "\"str\"", "'sym", "(hash 'a 1)"] {
        eval(&format!("(rng/uniform {junk})"));
    }
}

// Output seeds are signed integers that stand for the same seed.
#[test]
fn output_seeds_round_trip() {
    for key in 0..32 {
        let seed = gantz_rng::fold_int(Seed(7), key);
        let snippet = format!("(rng/uniform (rng/fold-in 7 {key}))");
        assert_eq!(eval(&snippet), SteelVal::NumV(gantz_rng::uniform(seed)));
    }
}

// Split gives no seeds for a count of 0 or less.
#[test]
fn split_of_no_seeds() {
    assert_same("(rng/split 7 0)", "'()");
    assert_same("(rng/split 7 -3)", "'()");
}

// Choose and shuffle accept vectors, and choose rejects an empty list.
#[test]
fn choose_and_shuffle_sequences() {
    assert_same("(rng/choose 7 #(a b c))", "(rng/choose 7 '(a b c))");
    assert_same(
        "(rng/choose 7 (vector 'a 'b 'c))",
        "(rng/choose 7 '(a b c))",
    );
    assert_same("(rng/shuffle 7 #(1 2 3))", "(rng/shuffle 7 '(1 2 3))");
    assert_same("(rng/shuffle 7 (vector 1 2 3))", "(rng/shuffle 7 '(1 2 3))");
    assert!(engine().run("(rng/choose 7 '())".to_string()).is_err());
    assert!(engine().run("(rng/choose 7 5)".to_string()).is_err());
    assert!(engine().run("(rng/bernoulli 7 'x)".to_string()).is_err());
}

// A deep nest of lists folds like its tokens. The walk uses an explicit
// stack, so the depth is limited only by memory.
#[test]
fn deep_nesting_folds() {
    const DEPTH: usize = 10_000;
    // Build and drop the nest on a thread with a large stack, as the drop
    // of a nested list recurses.
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(|| {
            let mut val = SteelVal::IntV(1);
            for _ in 0..DEPTH {
                val = SteelVal::ListV(vec![val].into());
            }
            let expected = (0..DEPTH).fold(Seed(7), |s, _| gantz_rng::fold_sequence(s, 1));
            let expected = gantz_rng::fold_int(expected, 1);
            assert_eq!(gantz_rng::steel::fold_value(Seed(7), &val), expected);
        })
        .unwrap()
        .join()
        .unwrap();
}
