//! The `~envgen` envelope model. It lives in `gantz_ui`, so the envelope
//! editor of the host edits it as data. This module adds what plyphon reads
//! of it: the shape codes of `EnvGen`, and the presets of SuperCollider's
//! `Env`.

pub use gantz_ui::envelope::{Envelope, Segment, Shape, shape_level};

/// The shape code of `shape` that plyphon's `EnvGen` reads.
pub fn shape_code(shape: Shape) -> f32 {
    match shape {
        Shape::Step => 0.0,
        Shape::Lin => 1.0,
        Shape::Exp => 2.0,
        Shape::Sin => 3.0,
        Shape::Welch => 4.0,
        Shape::Curve => 5.0,
        Shape::Squared => 6.0,
        Shape::Cubed => 7.0,
        Shape::Hold => 8.0,
    }
}

/// The envelope of a new `~envgen`, the ADSR of SuperCollider's `Env.adsr`.
pub fn initial() -> Envelope {
    adsr(0.01, 0.3, 0.5, 1.0)
}

/// An attack, decay, sustain and release envelope. It rises to 1, falls to
/// `sustain` and holds there while the gate is open.
pub fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Envelope {
    Envelope {
        init: 0.0,
        segments: vec![
            Segment::curved(1.0, attack, -4.0),
            Segment::curved(sustain, decay, -4.0),
            Segment::curved(0.0, release, -4.0),
        ],
        release: Some(2),
    }
}

/// A percussive envelope. It rises to 1 and falls to 0, with no sustain.
pub fn perc(attack: f32, release: f32) -> Envelope {
    Envelope {
        init: 0.0,
        segments: vec![
            Segment::curved(1.0, attack, -4.0),
            Segment::curved(0.0, release, -4.0),
        ],
        release: None,
    }
}

/// An attack, sustain and release envelope. It rises to `sustain` and holds
/// there while the gate is open.
pub fn asr(attack: f32, sustain: f32, release: f32) -> Envelope {
    Envelope {
        init: 0.0,
        segments: vec![
            Segment::curved(sustain, attack, -4.0),
            Segment::curved(0.0, release, -4.0),
        ],
        release: Some(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The code of a shape is its place in `Shape::ALL`, as `EnvGen` reads it.
    #[test]
    fn shape_codes_follow_envgen() {
        for (code, shape) in Shape::ALL.into_iter().enumerate() {
            assert_eq!(shape_code(shape), code as f32);
        }
    }
}
