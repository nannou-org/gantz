//! Steel-text rendering and parsing for [`Datum`]. `Datum` is the
//! self-describing serde value defined in [`gantz_core::datum`] and re-exported
//! at this crate's root.
//!
//! [`datum_text`] renders a `Datum` as a reader-valid Steel datum and
//! [`datum_from_expr`] reads one back from a parsed Steel expression.

use crate::sexpr::{self, list_args, quote, span_src};
pub use gantz_core::datum::{Datum, from_datum, node_datum};
use steel::parser::ast::{Atom, ExprKind};
use steel::parser::tokens::TokenType;

/// Render a [`Datum`] as a reader-valid Steel datum.
pub fn datum_text(d: &Datum) -> String {
    match d {
        Datum::Null => "null".to_string(),
        Datum::Bool(true) => "#t".to_string(),
        Datum::Bool(false) => "#f".to_string(),
        Datum::I64(n) => n.to_string(),
        Datum::U64(n) => n.to_string(),
        Datum::F64(x) => float_text(*x),
        Datum::Char(c) => char_text(*c),
        Datum::Str(s) => quote(s),
        Datum::Bytes(b) => {
            let inner = b.iter().map(u8::to_string).collect::<Vec<_>>().join(" ");
            format!("#u8({inner})")
        }
        Datum::Seq(items) => {
            let inner = items.iter().map(datum_text).collect::<Vec<_>>().join(" ");
            format!("#({inner})")
        }
        Datum::Map(entries) => {
            let inner = entries
                .iter()
                .map(|(k, v)| format!("({} {})", key_text(k), datum_text(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("({inner})")
        }
    }
}

/// Read a [`Datum`] from a Steel datum expression. Numbers are read from their
/// verbatim source slice in `src`. Seqs are vectors written `#(...)`. Maps are
/// bare lists of `(key value)` pairs.
pub fn datum_from_expr(e: &ExprKind, src: &str) -> Datum {
    match e {
        ExprKind::Vector(v) if v.bytes => {
            Datum::Bytes(v.args.iter().filter_map(|a| byte_of(a, src)).collect())
        }
        ExprKind::Vector(v) => Datum::Seq(v.args.iter().map(|a| datum_from_expr(a, src)).collect()),
        ExprKind::List(list) => Datum::Map(
            list.args
                .iter()
                .filter_map(|item| map_entry(item, src))
                .collect(),
        ),
        ExprKind::Atom(a) => atom_datum(a, e, src),
        _ => Datum::Null,
    }
}

fn map_entry(item: &ExprKind, src: &str) -> Option<(String, Datum)> {
    let args = list_args(item)?;
    if args.len() != 2 {
        return None;
    }
    let key = sexpr::as_symbol(&args[0]).or_else(|| sexpr::as_string(&args[0]))?;
    Some((key, datum_from_expr(&args[1], src)))
}

fn byte_of(e: &ExprKind, src: &str) -> Option<u8> {
    u8::try_from(sexpr::as_i64(e, src)?).ok()
}

fn atom_datum(a: &Atom, e: &ExprKind, src: &str) -> Datum {
    match &a.syn.ty {
        TokenType::StringLiteral(s) => Datum::Str(s.to_string()),
        TokenType::BooleanLiteral(b) => Datum::Bool(*b),
        TokenType::CharacterLiteral(c) => Datum::Char(*c),
        TokenType::Number(_) => number_datum(e, src),
        TokenType::Identifier(s) => match s.resolve() {
            "null" => Datum::Null,
            "true" => Datum::Bool(true),
            "false" => Datum::Bool(false),
            other => Datum::Str(other.to_string()),
        },
        TokenType::Keyword(s) => Datum::Str(s.resolve().to_string()),
        _ => Datum::Null,
    }
}

fn number_datum(e: &ExprKind, src: &str) -> Datum {
    let Some(text) = span_src(e, src) else {
        return Datum::Null;
    };
    if let Ok(i) = text.parse::<i64>() {
        Datum::I64(i)
    } else if let Ok(u) = text.parse::<u64>() {
        Datum::U64(u)
    } else if let Ok(f) = text.parse::<f64>() {
        Datum::F64(f)
    } else {
        Datum::Str(text.to_string())
    }
}

/// Render a float with a decimal point or exponent, so it never reads back as
/// an integer. `{:?}` gives the shortest round-tripping form.
fn float_text(x: f64) -> String {
    let s = format!("{x:?}");
    if s.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        s
    } else {
        format!("{s}.0")
    }
}

/// Render a `char` exactly as Steel's own reader displays it, so it round-trips.
fn char_text(c: char) -> String {
    match c {
        ' ' => "#\\space".to_string(),
        '\0' => "#\\null".to_string(),
        '\t' => "#\\tab".to_string(),
        '\n' => "#\\newline".to_string(),
        '\r' => "#\\return".to_string(),
        _ if c.escape_debug().count() == 1 => format!("#\\{c}"),
        _ => format!("#\\u{:04x}", c as u32),
    }
}

/// A map key is rendered as a bare symbol when it is identifier-safe, as struct
/// field names are. Otherwise it is a quoted string.
fn key_text(k: &str) -> String {
    let safe = !k.is_empty()
        && k.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if safe { k.to_string() } else { quote(k) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gantz_core::datum::to_datum;
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::fmt::Debug;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Inner {
        flag: bool,
        ratio: f64,
        tags: Vec<String>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(tag = "type")]
    enum MyNode {
        Scalar {
            count: u32,
            offset: i64,
            label: String,
        },
        Nested {
            inner: Inner,
            maybe: Option<u8>,
            extra: Vec<i32>,
        },
        Unit,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Meters(f64);

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Pair(i32, i32);

    /// An externally tagged enum, serde's default. It uses the single-key-map or
    /// bare string encoding, distinct from the internally tagged path `MyNode`
    /// takes.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    enum Shape {
        Dot,
        Tag(String),
        Span(i32, i32),
        Rect { w: u32, h: u32, fill: bool },
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        a: Option<i32>,
        b: Option<String>,
    }

    /// Reading a datum from text and rendering it back is stable.
    fn text_roundtrip(d: &Datum) -> Datum {
        let text = datum_text(d);
        let exprs = sexpr::read(&text).expect("read");
        assert_eq!(exprs.len(), 1, "one datum from `{text}`");
        datum_from_expr(&exprs[0], &text)
    }

    /// Assert a serde value survives the codec and a text round-trip.
    fn assert_serde_text_roundtrip<T>(value: &T)
    where
        T: Debug + PartialEq + Serialize + DeserializeOwned,
    {
        let datum = to_datum(value).expect("to_datum");
        let back: T = from_datum(text_roundtrip(&datum)).expect("from_datum after text");
        assert_eq!(back, *value, "text round-trip for {value:?}");
    }

    /// Datums that read back unchanged from their rendered text. A row with
    /// text also pins the rendered form.
    #[test]
    fn datums_round_trip_through_text() {
        let mut rows: Vec<(Datum, Option<&str>)> = vec![
            // Text-stable scalars.
            (Datum::Null, None),
            (Datum::Bool(true), None),
            (Datum::Bool(false), None),
            (Datum::I64(-42), None),
            (Datum::F64(3.5), None),
            // `3.0` must render with a decimal point so it does not read back
            // as I64.
            (Datum::F64(3.0), Some("3.0")),
            (Datum::Char('q'), None),
            (Datum::Str("hello world".to_string()), None),
            // Integer boundary values round-trip with the correct variant. A
            // value just past `i64::MAX` reads back as `U64`, not an overflow.
            (Datum::I64(i64::MIN), None),
            (Datum::I64(i64::MAX), None),
            (Datum::U64(i64::MAX as u64 + 1), None),
            // An empty map and an empty seq are distinct.
            (Datum::Map(vec![]), Some("()")),
            (Datum::Seq(vec![]), Some("#()")),
            (Datum::Bytes(vec![0, 1, 255]), Some("#u8(0 1 255)")),
            (Datum::Bytes(vec![]), Some("#u8()")),
            (
                Datum::Map(vec![
                    ("a".to_string(), Datum::I64(1)),
                    (
                        "b".to_string(),
                        Datum::Seq(vec![Datum::I64(2), Datum::Str("x".into())]),
                    ),
                    (
                        "c".to_string(),
                        Datum::Map(vec![("k".to_string(), Datum::Bool(true))]),
                    ),
                ]),
                None,
            ),
            // Empty collections nested inside collections stay distinct. A seq
            // holding an empty map, `#(())`, is not a seq holding an empty seq,
            // `#(#())`.
            (Datum::Seq(vec![Datum::Map(vec![])]), None),
            (Datum::Seq(vec![Datum::Seq(vec![])]), None),
            (
                Datum::Map(vec![
                    ("e_seq".into(), Datum::Seq(vec![])),
                    ("e_map".into(), Datum::Map(vec![])),
                ]),
                None,
            ),
            // A deeply mixed structure. It nests maps in seqs in maps and
            // interleaves nulls, bytes and strings with reader-significant
            // characters.
            (
                Datum::Map(vec![
                    (
                        "rows".into(),
                        Datum::Seq(vec![
                            Datum::Map(vec![
                                ("id".into(), Datum::I64(1)),
                                (
                                    "vals".into(),
                                    Datum::Seq(vec![Datum::F64(1.5), Datum::Null]),
                                ),
                            ]),
                            Datum::Map(vec![
                                ("id".into(), Datum::I64(2)),
                                ("vals".into(), Datum::Seq(vec![])),
                            ]),
                        ]),
                    ),
                    (
                        "grid".into(),
                        Datum::Seq(vec![
                            Datum::Seq(vec![Datum::I64(0), Datum::I64(1)]),
                            Datum::Seq(vec![Datum::Bool(true), Datum::Str("x".into())]),
                        ]),
                    ),
                    ("raw".into(), Datum::Bytes(vec![1, 2, 3])),
                    ("note".into(), Datum::Str("(parens) and \"quotes\"".into())),
                ]),
                None,
            ),
        ];
        // Special chars, then reader-significant chars via Steel's character
        // syntax. These cover parens, brackets, quotes, hash, the comment char
        // and a digit.
        let chars = [
            ' ', '\n', '\t', '\r', '\0', 'a', '✓', '\u{7}', '(', ')', '[', ']', '"', '\\', '#',
            ';', '5', '\'',
        ];
        rows.extend(chars.map(|c| (Datum::Char(c), None)));
        // Strings whose contents look like another datum kind must stay
        // strings. Quoting disambiguates them from `null`, `#t` and numbers on
        // read.
        let lookalikes = [
            "null", "true", "false", "#t", "#f", "42", "-7", "3.0", "-1.5", "1e9", "", " ",
        ];
        // Strings containing reader-significant characters survive quoting and
        // re-reading. The cases cover quotes, escapes, parens, comment and
        // keyword markers, and unicode.
        let escapes = [
            "a\"b",         // embedded double quote
            "a\\b",         // embedded backslash
            "line1\nline2", // newline
            "tab\there",    // tab
            "ret\rhere",    // carriage return
            "(not a list)", // parens inside a string
            "semi;colon",   // comment char
            "#:keyword",    // keyword marker
            "✓ unicode ☃",
            "",
        ];
        rows.extend(
            lookalikes
                .into_iter()
                .chain(escapes)
                .map(|s| (Datum::Str(s.to_string()), None)),
        );
        for (d, text) in &rows {
            if let Some(text) = text {
                assert_eq!(datum_text(d), *text, "text of {d:?}");
            }
            assert_eq!(
                text_roundtrip(d),
                *d,
                "text round-trip for {d:?} (rendered {:?})",
                datum_text(d),
            );
        }
    }

    /// A non-negative integer renders as bare digits, so it reads back as `I64`
    /// even when it was a `U64`. This is harmless. A node's field `Deserialize`
    /// produces the same value either way. The serde round-trip test compares
    /// nodes and is the real guard.
    #[test]
    fn nonnegative_int_normalizes_to_i64() {
        assert_eq!(text_roundtrip(&Datum::U64(42)), Datum::I64(42));
        assert_eq!(text_roundtrip(&Datum::I64(42)), Datum::I64(42));
        // A value beyond i64::MAX still round-trips as U64.
        let big = Datum::U64(u64::MAX);
        assert_eq!(text_roundtrip(&big), big);
    }

    /// Floats survive a text round-trip bit-exactly, including fractional,
    /// negative, very large/small, and full-precision values.
    #[test]
    fn floats_round_trip_exactly() {
        let cases = [
            0.0,
            -0.5,
            0.5,
            -3.5,
            0.1,
            0.1 + 0.2,
            1.0 / 3.0,
            1e-10,
            1e10,
            1e20,
            1e-20,
            123456.789,
            f64::MIN_POSITIVE,
        ];
        for x in cases {
            match text_roundtrip(&Datum::F64(x)) {
                Datum::F64(y) => assert_eq!(
                    x.to_bits(),
                    y.to_bits(),
                    "float {x} must round-trip exactly (rendered {:?})",
                    float_text(x),
                ),
                other => panic!(
                    "float {x} read back as {other:?} (rendered {:?})",
                    float_text(x)
                ),
            }
        }
    }

    /// Serde values survive the datum codec and a text round-trip.
    #[test]
    fn serde_values_round_trip_through_text() {
        let nested = MyNode::Nested {
            inner: Inner {
                flag: true,
                ratio: 0.25,
                tags: vec!["a".into(), "b".into()],
            },
            maybe: Some(7),
            extra: vec![-1, 0, 1],
        };
        // In-memory codec is exact.
        let back: MyNode = from_datum(to_datum(&nested).expect("to_datum")).expect("from_datum");
        assert_eq!(nested, back, "in-memory round-trip");
        // Internally tagged struct, unit and scalar variants.
        for node in [
            nested,
            MyNode::Unit,
            MyNode::Scalar {
                count: 3,
                offset: -9,
                label: "hi".into(),
            },
        ] {
            assert_serde_text_roundtrip(&node);
        }
        // Every externally tagged variant shape round-trips. This exercises the
        // enum and variant-access paths for unit, newtype, tuple and struct
        // variants.
        for shape in [
            Shape::Dot,
            Shape::Tag("hi".into()),
            Shape::Span(-1, 2),
            Shape::Rect {
                w: 4,
                h: 5,
                fill: true,
            },
        ] {
            assert_serde_text_roundtrip(&shape);
        }
        // Tuple structs, newtype structs and tuples round-trip as sequences.
        assert_serde_text_roundtrip(&Meters(2.5));
        assert_serde_text_roundtrip(&Pair(-3, 7));
        assert_serde_text_roundtrip(&(1u8, "two".to_string(), 3.5f64));
        // Maps with non-identifier string keys and with numeric keys
        // round-trip, exercising the map-key serializer and deserializer.
        let str_keys: BTreeMap<String, i32> = [
            ("plain".to_string(), 1),
            ("with space".to_string(), 2),
            ("123".to_string(), 3), // digit-leading, must be quoted
            (String::new(), 4),     // empty key, must be quoted
            ("dash-ok".to_string(), 5),
        ]
        .into_iter()
        .collect();
        assert_serde_text_roundtrip(&str_keys);
        let num_keys: BTreeMap<i32, String> = [(-2, "neg".to_string()), (7, "pos".to_string())]
            .into_iter()
            .collect();
        assert_serde_text_roundtrip(&num_keys);
        // `None`, the null datum, round-trips and stays distinct from a present
        // value.
        assert_serde_text_roundtrip(&Holder {
            a: None,
            b: Some("x".into()),
        });
        assert_serde_text_roundtrip(&Holder {
            a: Some(0),
            b: None,
        });
    }

    /// Deserializing a datum into an incompatible type fails cleanly rather than
    /// silently coercing.
    #[test]
    fn type_mismatch_is_an_error() {
        assert!(from_datum::<u8>(Datum::I64(300)).is_err()); // out of range
        assert!(from_datum::<i64>(Datum::Str("nope".into())).is_err());
        assert!(from_datum::<String>(Datum::I64(1)).is_err());
        assert!(from_datum::<bool>(Datum::Null).is_err());
    }
}
