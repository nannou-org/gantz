//! Decoding `pat/plot-data` output into plottable channels.
//!
//! Event values are classified into [`Leaf`]s. A map value splits into one
//! channel per key and a list or vector value into one channel per index.
//! Only the top level splits. A container nested within becomes opaque.

use gantz_core::steel::SteelVal;
use num_traits::ToPrimitive;
use std::collections::BTreeMap;

/// A plottable value.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Leaf {
    /// A number, or a bool as 1 or 0. Plotted on the value axis.
    Num(f64),
    /// A string, symbol or char. Plotted as a labelled span.
    Label(String),
    /// Any other value, with its display text. Plotted as a bare span.
    Opaque(String),
}

/// The channel that an event value, or a part of one, plots in.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ChannelKey {
    /// A whole value that is neither a map nor a list.
    Whole,
    /// An element of a list or vector value.
    Index(usize),
    /// An entry of a map value, by the key's text.
    Key(String),
}

/// A discrete event's active part in one channel.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Seg {
    pub start: f64,
    pub end: f64,
    pub leaf: Leaf,
    pub onset: bool,
}

/// One channel's events and signal samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Channel {
    /// One per discrete event.
    pub segments: Vec<Seg>,
    /// The `[x, y]` samples of continuous numeric signals.
    pub points: Vec<[f64; 2]>,
}

/// The decoded plot state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PlotData {
    /// The plotted span, `[start, end]`, in cycles.
    pub span: [f64; 2],
    /// The channels in key order.
    pub channels: BTreeMap<ChannelKey, Channel>,
}

impl Leaf {
    /// The value on the value axis, for a numeric leaf.
    pub fn num(&self) -> Option<f64> {
        match self {
            Leaf::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// The text shown for the value on hover.
    pub fn text(&self) -> String {
        match self {
            Leaf::Num(n) => format!("{n}"),
            Leaf::Label(s) | Leaf::Opaque(s) => s.clone(),
        }
    }
}

impl ChannelKey {
    /// The key's text. The decimal index for a list element. `None` for a
    /// whole value.
    pub fn text(&self) -> Option<String> {
        match self {
            ChannelKey::Whole => None,
            ChannelKey::Index(i) => Some(i.to_string()),
            ChannelKey::Key(k) => Some(k.clone()),
        }
    }
}

impl PlotData {
    /// The total number of events across all channels.
    pub fn n_events(&self) -> usize {
        self.channels.values().map(|c| c.segments.len()).sum()
    }
}

/// Decode `pat/plot-data` output. `None` unless the value is the expected
/// `(start end segments points)` list. Malformed entries are skipped.
pub(super) fn plot_data(val: &SteelVal) -> Option<PlotData> {
    let [start, end, segments, points] = list_n(val)?;
    let span = [num(&start)?, num(&end)?];
    let mut channels: BTreeMap<ChannelKey, Channel> = BTreeMap::new();
    for seg in list(&segments)? {
        let Some([start, end, value, onset]) = list_n(&seg) else {
            continue;
        };
        let (Some(start), Some(end)) = (num(&start), num(&end)) else {
            continue;
        };
        let onset = matches!(onset, SteelVal::BoolV(true));
        for (key, leaf) in channels_of(&value) {
            let seg = Seg {
                start,
                end,
                leaf,
                onset,
            };
            channels.entry(key).or_default().segments.push(seg);
        }
    }
    for pt in list(&points)? {
        let Some([x, value]) = list_n(&pt) else {
            continue;
        };
        let Some(x) = num(&x) else {
            continue;
        };
        // A sample has no width to draw as a span, so only numbers plot.
        for (key, leaf) in channels_of(&value) {
            if let Some(y) = leaf.num() {
                channels.entry(key).or_default().points.push([x, y]);
            }
        }
    }
    Some(PlotData { span, channels })
}

/// Split an event value into its channels and their leaves. A map gives one
/// channel per entry, and a list or vector one per element. Anything else is
/// one whole leaf.
pub(super) fn channels_of(val: &SteelVal) -> Vec<(ChannelKey, Leaf)> {
    match val {
        SteelVal::HashMapV(map) => map
            .iter()
            .map(|(k, v)| (ChannelKey::Key(key_text(k)), leaf(v)))
            .collect(),
        SteelVal::ListV(l) => indexed(l.iter()),
        SteelVal::VectorV(v) => indexed(v.iter()),
        other => vec![(ChannelKey::Whole, leaf(other))],
    }
}

/// Classify a single value. Containers are opaque here, so only the top level
/// of an event value splits into channels.
pub(super) fn leaf(val: &SteelVal) -> Leaf {
    match val {
        SteelVal::BoolV(b) => Leaf::Num(if *b { 1.0 } else { 0.0 }),
        SteelVal::StringV(s) | SteelVal::SymbolV(s) => Leaf::Label(s.to_string()),
        SteelVal::CharV(c) => Leaf::Label(c.to_string()),
        other => match num(other) {
            Some(n) => Leaf::Num(n),
            None => Leaf::Opaque(other.to_string()),
        },
    }
}

fn indexed<'a>(vals: impl Iterator<Item = &'a SteelVal>) -> Vec<(ChannelKey, Leaf)> {
    vals.enumerate()
        .map(|(i, v)| (ChannelKey::Index(i), leaf(v)))
        .collect()
}

/// A map key's text. A symbol or string is its name. Anything else is its
/// display text.
fn key_text(key: &SteelVal) -> String {
    match key {
        SteelVal::StringV(s) | SteelVal::SymbolV(s) => s.to_string(),
        other => other.to_string(),
    }
}

/// Any real numeric value as `f64`.
fn num(val: &SteelVal) -> Option<f64> {
    match val {
        SteelVal::NumV(f) => Some(*f),
        SteelVal::IntV(i) => i.to_f64(),
        SteelVal::Rational(r) => r.to_f64(),
        SteelVal::BigNum(b) => b.to_f64(),
        SteelVal::BigRational(r) => r.to_f64(),
        _ => None,
    }
}

/// The elements of a list value.
fn list(val: &SteelVal) -> Option<Vec<SteelVal>> {
    match val {
        SteelVal::ListV(l) => Some(l.iter().cloned().collect()),
        _ => None,
    }
}

/// The elements of a list value of exactly `N` elements.
fn list_n<const N: usize>(val: &SteelVal) -> Option<[SteelVal; N]> {
    list(val)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num_v(n: f64) -> SteelVal {
        SteelVal::NumV(n)
    }

    fn list_v(xs: Vec<SteelVal>) -> SteelVal {
        SteelVal::ListV(xs.into_iter().collect())
    }

    fn sym(s: &str) -> SteelVal {
        SteelVal::SymbolV(s.into())
    }

    // Each value kind classifies to its leaf.
    #[test]
    fn leaves_by_kind() {
        assert_eq!(leaf(&num_v(1.5)), Leaf::Num(1.5));
        assert_eq!(leaf(&SteelVal::IntV(3)), Leaf::Num(3.0));
        let half = num_rational::Rational32::new(1, 2);
        assert_eq!(leaf(&SteelVal::Rational(half)), Leaf::Num(0.5));
        assert_eq!(leaf(&SteelVal::BoolV(true)), Leaf::Num(1.0));
        assert_eq!(leaf(&SteelVal::BoolV(false)), Leaf::Num(0.0));
        assert_eq!(leaf(&sym("bd")), Leaf::Label("bd".into()));
        assert_eq!(
            leaf(&SteelVal::StringV("hi".into())),
            Leaf::Label("hi".into())
        );
        assert_eq!(leaf(&SteelVal::CharV('x')), Leaf::Label("x".into()));
        assert!(matches!(leaf(&SteelVal::Void), Leaf::Opaque(_)));
        // A nested container is opaque.
        assert!(matches!(leaf(&list_v(vec![num_v(1.0)])), Leaf::Opaque(_)));
    }

    // Lists split by index. Other values are whole.
    #[test]
    fn channels_by_shape() {
        assert_eq!(
            channels_of(&num_v(2.0)),
            vec![(ChannelKey::Whole, Leaf::Num(2.0))],
        );
        assert_eq!(
            channels_of(&list_v(vec![num_v(1.0), sym("a")])),
            vec![
                (ChannelKey::Index(0), Leaf::Num(1.0)),
                (ChannelKey::Index(1), Leaf::Label("a".into())),
            ],
        );
    }

    // Channels order whole first, then by index, then by key text.
    #[test]
    fn channel_key_order() {
        let mut keys = vec![
            ChannelKey::Key("s".into()),
            ChannelKey::Index(1),
            ChannelKey::Key("n".into()),
            ChannelKey::Whole,
            ChannelKey::Index(0),
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                ChannelKey::Whole,
                ChannelKey::Index(0),
                ChannelKey::Index(1),
                ChannelKey::Key("n".into()),
                ChannelKey::Key("s".into()),
            ],
        );
    }

    // Decoding rejects a malformed state and skips malformed entries. Only
    // numeric signal samples plot.
    #[test]
    fn plot_data_decodes_totally() {
        assert_eq!(plot_data(&list_v(vec![])), None);
        assert_eq!(plot_data(&num_v(1.0)), None);
        let state = list_v(vec![
            num_v(0.0),
            num_v(1.0),
            list_v(vec![
                list_v(vec![
                    num_v(0.0),
                    num_v(0.5),
                    sym("bd"),
                    SteelVal::BoolV(true),
                ]),
                list_v(vec![num_v(0.0)]),
                list_v(vec![
                    num_v(0.5),
                    num_v(1.0),
                    num_v(3.0),
                    SteelVal::BoolV(false),
                ]),
            ]),
            list_v(vec![
                list_v(vec![num_v(0.25), num_v(0.5)]),
                list_v(vec![num_v(0.75), sym("x")]),
                num_v(9.0),
            ]),
        ]);
        let data = plot_data(&state).expect("plot data");
        assert_eq!(data.span, [0.0, 1.0]);
        assert_eq!(data.n_events(), 2);
        let whole = &data.channels[&ChannelKey::Whole];
        assert_eq!(
            whole.segments,
            vec![
                Seg {
                    start: 0.0,
                    end: 0.5,
                    leaf: Leaf::Label("bd".into()),
                    onset: true,
                },
                Seg {
                    start: 0.5,
                    end: 1.0,
                    leaf: Leaf::Num(3.0),
                    onset: false,
                },
            ],
        );
        assert_eq!(whole.points, vec![[0.25, 0.5]]);
    }
}
