//! Shared parts of the DSP nodes' views. See [`gantz_nodedef::View`].
//!
//! A param's value lives in the node's state, so its dialer binds the
//! node's own state with an `at` key path and edits no field. Lags, counts,
//! shapes and rates are fields, so their controls send edits that the node
//! applies. See [`crate::param`].

use crate::dsp::NodeRate;
use gantz_ca::Datum;
use gantz_nodedef::{Row, SocketDoc};
use gantz_ui::{self as ui, At, BindPath, Edit, Element};

/// The edit of a node's rate.
pub(crate) const RATE: &str = "rate";

/// The prefix of the edit of a param's smoothing lag, as in `lag:freq`.
pub(crate) const LAG: &str = "lag:";

/// The longest smoothing lag of a param in seconds.
pub(crate) const MAX_LAG: f32 = 10.0;

/// The edit of a node's socket count.
pub(crate) const COUNT: &str = "count";

/// The most sockets that a node with a socket count has.
pub(crate) const MAX_COUNT: usize = 64;

/// The doc of a socket whose values have the type `ty`.
pub(crate) fn doc(ty: &str, description: impl Into<String>) -> SocketDoc {
    SocketDoc::new(ty, &description.into())
}

/// A row of one line in the inspector's table.
pub(crate) fn row(label: &str, ui: Element) -> Row {
    lines_row(label, ui, 1)
}

/// A row of the inspector `lines` lines high.
pub(crate) fn lines_row(label: &str, ui: Element, lines: u32) -> Row {
    Row {
        label: label.into(),
        ui,
        lines,
    }
}

/// A dialer that shows `value` and sends `edit` with its new value.
pub(crate) fn edit_dialer(edit: &str, value: f64) -> ui::Dialer {
    ui::Dialer {
        on: Some(Edit(edit.to_string())),
        value: Some(value),
        ..ui::Dialer::default()
    }
}

/// The row of a param whose value lives in the node's state at the keys
/// `at`, beside a dialer for its smoothing `lag` that sends the `lag:<key>`
/// edit. The value dialer ranges over `min..=max` with the unit `suffix`.
pub(crate) fn param_row(
    name: &str,
    at: Vec<String>,
    [min, max]: [f32; 2],
    suffix: &str,
    lag: f32,
) -> Row {
    let step = ((max - min) as f64 / 2_000.0).max(0.000_5);
    let value = ui::Dialer {
        bind: Some(BindPath(vec![])),
        at: Some(At(at)),
        min: Some(min.into()),
        max: Some(max.into()),
        step: Some(step),
        prefix: (!suffix.is_empty()).then(|| format!("{} ", suffix.trim())),
        push: false,
        tip: Some(format!("the {name}. A connected number overrides it")),
        ..ui::Dialer::default()
    };
    let lag = ui::Dialer {
        min: Some(0.0),
        max: Some(MAX_LAG.into()),
        step: Some(0.001),
        precision: Some(3),
        label: Some("s lag".into()),
        tip: Some("one-pole smoothing time in seconds, 0 is instant".into()),
        ..edit_dialer(&format!("{LAG}{name}"), lag.into())
    };
    let children = vec![Element::Dialer(value), Element::Dialer(lag)];
    row(
        name,
        Element::Row(ui::Row {
            children,
            ..ui::Row::default()
        }),
    )
}

/// The row that picks a node's rate. A change derives a new synthdef.
pub(crate) fn rate_row(rate: NodeRate) -> Row {
    let choice = ui::Choice {
        on: Some(Edit(RATE.into())),
        value: Some(rate.token().into()),
        options: vec!["ar".into(), "kr".into()],
        tips: vec![
            "audio rate, one value per sample".into(),
            "control rate, one value per block. Cheaper, for modulators".into(),
        ],
        ..ui::Choice::default()
    };
    row("rate", Element::Choice(choice))
}

/// The rate that a `rate` edit's value names.
pub(crate) fn edit_rate(value: &Datum) -> Option<NodeRate> {
    match value.as_str()? {
        "ar" => Some(NodeRate::Audio),
        "kr" => Some(NodeRate::Control),
        _ => None,
    }
}

/// The whole number in `min..=max` nearest an edit's numeric value.
pub(crate) fn edit_count(value: &Datum, [min, max]: [usize; 2]) -> Option<usize> {
    let n = value.as_f64().filter(|n| n.is_finite())?;
    Some((n.round().max(0.0) as usize).clamp(min, max))
}

/// A non-negative number of seconds from an edit's numeric value, at most
/// `max`.
pub(crate) fn edit_secs(value: &Datum, max: f32) -> Option<f32> {
    let n = value.as_f64().filter(|n| n.is_finite())?;
    Some((n as f32).clamp(0.0, max))
}

/// The row of a node's socket count. A change changes the node's sockets.
pub(crate) fn count_row(count: usize, tip: &str) -> Row {
    let dialer = ui::Dialer {
        min: Some(1.0),
        max: Some(MAX_COUNT as f64),
        step: Some(1.0),
        precision: Some(0),
        tip: Some(tip.to_string()),
        ..edit_dialer(COUNT, count as f64)
    };
    row("count", Element::Dialer(dialer))
}

/// The row of a param with no smoothing, whose value lives in the node's
/// state at the keys `at`.
pub(crate) fn state_row(name: &str, at: Vec<String>, tip: &str) -> Row {
    let value = ui::Dialer {
        bind: Some(BindPath(vec![])),
        at: Some(At(at)),
        step: Some(0.01),
        push: false,
        tip: Some(tip.to_string()),
        ..ui::Dialer::default()
    };
    row(name, Element::Dialer(value))
}

/// A toggle that shows `value` and sends `edit` with its new value.
pub(crate) fn edit_toggle(edit: &str, value: bool, tip: &str) -> ui::Toggle {
    ui::Toggle {
        on: Some(Edit(edit.to_string())),
        value,
        label: Some(edit.to_string()),
        pill: true,
        tip: Some(tip.to_string()),
        ..ui::Toggle::default()
    }
}
