//! Euclidean rhythm tests: the full bjorklund onset table ported from
//! cycles' `bjorklund.rs`, plus event-level euclid/euclid-off/euclid-full
//! behavior.

mod common;

use common::{assert_pinned, eval_in, new_pin_engine};
use gantz_core::steel::SteelVal;

/// Every pinned (k, n, onsets) vector from cycles' bjorklund tests,
/// onsets encoded as one char per slot ('t' onset, 'f' rest).
const TABLE: &[(usize, usize, &str)] = &[
    (2, 3, "ttf"),
    (2, 5, "tftff"),
    (3, 4, "tttf"),
    (3, 5, "tftft"),
    (3, 8, "tfftfftf"),
    (4, 7, "tftftft"),
    (4, 9, "tftftftff"),
    (4, 12, "tfftfftfftff"),
    (4, 15, "tffftffftffftff"),
    (5, 6, "tttttf"),
    (5, 7, "tfttftt"),
    (5, 8, "tfttfttf"),
    (5, 9, "tftftftft"),
    (5, 11, "tftftftftff"),
    (5, 12, "tfftftfftftf"),
    (5, 13, "tfftftfftftff"),
    (5, 16, "tfftfftfftfftfff"),
    (6, 7, "ttttttf"),
    (6, 13, "tftftftftftff"),
    (7, 8, "tttttttf"),
    (7, 9, "tftttfttt"),
    (7, 10, "tfttfttftt"),
    (7, 12, "tfttftfttftf"),
    (7, 15, "tftftftftftftff"),
    (7, 16, "tfftftftfftftftf"),
    (7, 17, "tfftftfftftfftftf"),
    (7, 18, "tfftftfftftfftftff"),
    (8, 17, "tftftftftftftftff"),
    (8, 19, "tfftftftfftftftfftf"),
    (9, 14, "tfttfttfttfttf"),
    (9, 16, "tfttftftfttftftf"),
    (9, 22, "tfftftfftftfftftfftftf"),
    (9, 23, "tfftftfftftfftftfftftff"),
    (11, 12, "tttttttttttf"),
    (11, 24, "tfftftftftftfftftftftftf"),
    (13, 24, "tfttftftftftfttftftftftf"),
    (15, 34, "tfftftftftfftftftftfftftftftfftftf"),
];

/// Render a "tf" onset string as a steel boolean-list literal.
fn steel_bools(onsets: &str) -> String {
    let bools: Vec<&str> = onsets
        .chars()
        .map(|c| match c {
            't' => "#t",
            'f' => "#f",
            other => panic!("bad onset char {other:?}"),
        })
        .collect();
    format!("(list {})", bools.join(" "))
}

// The full onset table from cycles' bjorklund tests, on one shared engine.
#[test]
fn bjorklund_table() {
    let mut vm = new_pin_engine();
    for &(k, n, onsets) in TABLE {
        assert_eq!(onsets.len(), n, "bad table row ({k}, {n})");
        assert_eq!(
            onsets.matches('t').count(),
            k,
            "bad onset count in table row ({k}, {n})",
        );
        let check = format!(
            "(equal? {} (pat/euclid-bools {k} {n} 0))",
            steel_bools(onsets),
        );
        match eval_in(&mut vm, &check) {
            SteelVal::BoolV(true) => (),
            other => panic!("bjorklund ({k}, {n}) mismatch: {other:?}"),
        }
    }
}

// euclid 3 8: onsets at slots 0, 3 and 6, each one slot long.
#[test]
fn euclid_3_8_events() {
    assert_pinned(
        "((#t ((0 1) (1 8)) ((0 1) (1 8))) \
          (#t ((3 8) (1 2)) ((3 8) (1 2))) \
          (#t ((3 4) (7 8)) ((3 4) (7 8))))",
        "(pin-events (pat/query (pat/euclid 3 8) (pat/span 0 1)))",
    );
}

// A rotation of one slot moves the onsets left.
#[test]
fn euclid_off_rotates() {
    assert_pinned(
        "((#t ((1 4) (3 8)) ((1 4) (3 8))) \
          (#t ((5 8) (3 4)) ((5 8) (3 4))) \
          (#t ((7 8) (1 1)) ((7 8) (1 1))))",
        "(pin-events (pat/query (pat/euclid-off 3 8 1) (pat/span 0 1)))",
    );
}

// euclid-full elongates each onset to fill the silence before the next.
#[test]
fn euclid_full_elongates() {
    assert_pinned(
        "((#t ((0 1) (3 8)) ((0 1) (3 8))) \
          (#t ((3 8) (3 4)) ((3 8) (3 4))) \
          (#t ((3 4) (1 1)) ((3 4) (1 1))))",
        "(pin-events (pat/query (pat/euclid-full 3 8) (pat/span 0 1)))",
    );
}

// No onsets yields silence from every variant.
#[test]
fn euclid_zero_onsets() {
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/euclid 0 8) (pat/span 0 1)))",
    );
    assert_pinned(
        "()",
        "(pin-events (pat/query (pat/euclid-full 0 8) (pat/span 0 1)))",
    );
}
