//! The descriptor table behind [`UnitNode`](crate::UnitNode), one
//! [`UnitDesc`] row per wrapped plyphon unit generator.
//!
//! plyphon's [`Unit`](plyphon::Unit) and [`UnitDef`](plyphon::UnitDef) traits
//! expose no metadata. A unit's arity, input roles and defaults live only in
//! its docs. This table is where gantz declares each wrapped unit's
//! signature. A row holds the palette and sugar keyword, the inputs in
//! plyphon order with how each is fed, see [`In`], and the outputs. Adding a
//! plyphon unit as a gantz node is one new row.
//!
//! The operator-selector units `BinaryOpUGen` and `UnaryOpUGen` get one row
//! per operator. Such a row carries a [`Special`] override naming the real
//! emitted unit and its `special_index`, while its [`unit`](UnitDesc::unit)
//! field holds a unique per-operator identity such as `"Mul"` or `"TanH"`.
//!
//! The table excludes buffer-reading units, variable-arity units such as
//! `EnvGen` and `Klang`, demand-rate units, FFT/PV units and IO/routing
//! units. The bespoke nodes cover IO and routing.
//!
//! Most rows run at either rate, chosen on the node. A row whose unit only
//! makes sense at one rate, such as the `A2K`/`K2A` converters and the
//! engine info units, carries a [`UnitRate::Fixed`] constraint. plyphon
//! accepts any rate at build time and misbehaves silently at the wrong one,
//! so the constraint lives here.

use crate::dsp::NodeRate;

/// How one plyphon input of a wrapped unit is fed.
///
/// `Signal` and `Param` entries are sockets, dsp input ports in entry order.
/// `Baked` and `Init` entries are socket-less constants. Entry order matches
/// the unit's plyphon input order.
#[derive(Clone, Copy, Debug)]
pub enum In {
    /// A pure dsp input, a socket carrying a signal. Unconnected, it reads as
    /// [`input_or_silent`](crate::dsp::input_or_silent).
    Signal {
        /// The input's name, for socket docs.
        name: &'static str,
        /// The socket's doc line.
        doc: &'static str,
    },
    /// A hybrid input, see [`NodeDsp::n_dsp_inputs`](crate::NodeDsp). A
    /// connected signal drives the socket directly. Otherwise it falls back
    /// to a settable control param. The param's value lives in the node's
    /// keyed VM state, see [`param`](crate::param). Its smoothing lag lives in
    /// the node weight.
    Param {
        /// The param's name, its VM-state key, inspector label and sugar stem.
        name: &'static str,
        /// The value a fresh node starts at.
        default: f32,
        /// The inspector's drag range minimum.
        min: f32,
        /// The inspector's drag range maximum.
        max: f32,
        /// The inspector's unit suffix such as `" Hz"`, possibly empty.
        suffix: &'static str,
        /// The socket and param doc line.
        doc: &'static str,
    },
    /// A fixed constant with no socket, for example an initial phase the node
    /// does not expose.
    Baked(f32),
    /// An init-only structural value with no socket, baked into the def as a
    /// constant from the node weight. It is for inputs plyphon requires to be
    /// compile-time constants, such as a delay's `maxdelay` that sizes its
    /// delay line, or latches at unit init, such as `Line`. Editing one
    /// re-derives the synthdef.
    Init {
        /// The value's name, its inspector label and sugar keyword.
        name: &'static str,
        /// The value a fresh node starts at.
        default: f32,
        /// The inspector row's doc line.
        doc: &'static str,
    },
}

impl In {
    /// The entry's socket or inspector name. `Baked` has none.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            In::Signal { name, .. } | In::Param { name, .. } | In::Init { name, .. } => Some(name),
            In::Baked(_) => None,
        }
    }

    /// Whether this entry is a socket, a dsp input port.
    pub fn is_socket(&self) -> bool {
        matches!(self, In::Signal { .. } | In::Param { .. })
    }
}

/// A [`UnitDesc`] emission override for scsynth's operator-selector units.
/// It names the real emitted plyphon unit and the `special_index` selecting
/// the operator. A row carrying one keeps a unique per-operator
/// [`unit`](UnitDesc::unit) identity such as `"Mul"` or `"TanH"`, which is
/// never a plyphon registry name.
#[derive(Clone, Copy, Debug)]
pub struct Special {
    /// The emitted plyphon unit name, `"BinaryOpUGen"` or `"UnaryOpUGen"`.
    pub unit: &'static str,
    /// The operator selector, scsynth's `mSpecialIndex`.
    pub index: i16,
}

/// The ugen rates a descriptor row may run at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitRate {
    /// Either rate. The node weight carries the choice, `ar` when absent.
    Any,
    /// Exactly one rate. The weight carries no choice, the inspector shows
    /// no rate row and sugar never writes `#:rate`.
    Fixed(NodeRate),
}

/// One wrapped plyphon unit generator, the descriptor that drives a
/// [`UnitNode`](crate::UnitNode). The node names it by [`unit`](Self::unit).
#[derive(Clone, Copy, Debug)]
pub struct UnitDesc {
    /// The `.gantz` keyword and palette name, for example `"~lpf"`.
    pub keyword: &'static str,
    /// The row's unique identity and the node's stored `unit` field, for
    /// example `"LPF"`. Unless [`special`](Self::special) overrides it, this
    /// is also the emitted [`UnitSpec`](plyphon::synthdef::UnitSpec) name.
    pub unit: &'static str,
    /// The emission override for operator-selector rows. `None` means
    /// [`unit`](Self::unit) is itself the emitted plyphon name.
    pub special: Option<Special>,
    /// The rates the unit may run at.
    pub rate: UnitRate,
    /// One entry per plyphon input, in plyphon input order.
    pub inputs: &'static [In],
    /// One doc line per unit output (the node's dsp output ports).
    pub outputs: &'static [&'static str],
    /// The palette/inspector description.
    pub doc: &'static str,
}

impl UnitDesc {
    /// The emitted plyphon unit name, from the [`Special`] override when
    /// there is one and from [`unit`](Self::unit) otherwise.
    pub fn emitted_unit(&self) -> &'static str {
        match self.special {
            Some(Special { unit, .. }) => unit,
            None => self.unit,
        }
    }

    /// The emitted `special_index`. Non-operator rows emit `0`.
    pub fn special_index(&self) -> i16 {
        match self.special {
            Some(Special { index, .. }) => index,
            None => 0,
        }
    }

    /// The rate a fresh node of this row runs at. The fixed rate for a
    /// [`UnitRate::Fixed`] row, else `ar`.
    pub fn default_rate(&self) -> NodeRate {
        match self.rate {
            UnitRate::Fixed(rate) => rate,
            UnitRate::Any => NodeRate::default(),
        }
    }

    /// The `Signal` and `Param` entries in socket order.
    pub fn sockets(&self) -> impl Iterator<Item = &'static In> + '_ {
        self.inputs.iter().filter(|i| i.is_socket())
    }

    /// The number of dsp input sockets.
    pub fn n_sockets(&self) -> usize {
        self.sockets().count()
    }

    /// The hybrid params as `(name, default)`, in socket order.
    pub fn hybrid_params(&self) -> impl Iterator<Item = (&'static str, f32)> + '_ {
        self.inputs.iter().filter_map(|i| match i {
            In::Param { name, default, .. } => Some((*name, *default)),
            _ => None,
        })
    }

    /// The hybrid params as `(socket index, name)`, for wiring each hybrid
    /// socket to its VM-state key.
    pub fn hybrid_sockets(&self) -> impl Iterator<Item = (usize, &'static str)> + '_ {
        self.sockets().enumerate().filter_map(|(ix, i)| match i {
            In::Param { name, .. } => Some((ix, *name)),
            _ => None,
        })
    }

    /// The init-only entries as `(name, default)`.
    pub fn init_params(&self) -> impl Iterator<Item = (&'static str, f32)> + '_ {
        self.inputs.iter().filter_map(|i| match i {
            In::Init { name, default, .. } => Some((*name, *default)),
            _ => None,
        })
    }

    /// The default value of the `name`d init-only entry, if any.
    pub fn init_default(&self, name: &str) -> Option<f32> {
        self.init_params()
            .find_map(|(n, d)| (n == name).then_some(d))
    }

    /// The `ix`th input socket's doc line.
    pub fn socket_doc(&self, ix: usize) -> Option<&'static str> {
        self.sockets().nth(ix).map(|i| match i {
            In::Signal { doc, .. } | In::Param { doc, .. } => *doc,
            _ => unreachable!("sockets() yields only socketed entries"),
        })
    }
}

/// The descriptor for the plyphon unit named `unit`, if wrapped.
pub fn unit_desc(unit: &str) -> Option<&'static UnitDesc> {
    UNITS.iter().find(|d| d.unit == unit)
}

/// The descriptor with the given `.gantz` keyword such as `"~lpf"`, if any.
pub fn unit_desc_by_keyword(keyword: &str) -> Option<&'static UnitDesc> {
    UNITS.iter().find(|d| d.keyword == keyword)
}

/// A [`In::Signal`] row entry.
const fn sig(name: &'static str, doc: &'static str) -> In {
    In::Signal { name, doc }
}

/// A hybrid [`In::Param`] row entry.
const fn par(
    name: &'static str,
    default: f32,
    min: f32,
    max: f32,
    suffix: &'static str,
    doc: &'static str,
) -> In {
    In::Param {
        name,
        default,
        min,
        max,
        suffix,
        doc,
    }
}

/// A [`In::Baked`] row entry.
const fn baked(value: f32) -> In {
    In::Baked(value)
}

/// An [`In::Init`] row entry.
const fn init(name: &'static str, default: f32, doc: &'static str) -> In {
    In::Init { name, default, doc }
}

/// A [`UnitDesc`] row.
const fn u(
    keyword: &'static str,
    unit: &'static str,
    inputs: &'static [In],
    outputs: &'static [&'static str],
    doc: &'static str,
) -> UnitDesc {
    UnitDesc {
        keyword,
        unit,
        special: None,
        rate: UnitRate::Any,
        inputs,
        outputs,
        doc,
    }
}

/// Constrain a row to audio rate.
const fn ar_only(desc: UnitDesc) -> UnitDesc {
    UnitDesc {
        rate: UnitRate::Fixed(NodeRate::Audio),
        ..desc
    }
}

/// Constrain a row to control rate.
const fn kr_only(desc: UnitDesc) -> UnitDesc {
    UnitDesc {
        rate: UnitRate::Fixed(NodeRate::Control),
        ..desc
    }
}

/// A binary-operator row. It emits a `BinaryOpUGen` selecting the operator
/// at the given `special_index`, with a pure signal input `a` and a hybrid
/// param input `b`. This is a macro rather than a `const fn` so that each
/// row's input slice is a promotable literal.
macro_rules! bop {
    ($kw:literal, $unit:literal, $ix:literal, $b:literal, $b_doc:literal, $out:literal, $doc:literal $(,)?) => {
        UnitDesc {
            keyword: $kw,
            unit: $unit,
            special: Some(Special {
                unit: "BinaryOpUGen",
                index: $ix,
            }),
            rate: UnitRate::Any,
            inputs: &[
                sig("a", "left operand signal"),
                par("b", $b, -10_000.0, 10_000.0, "", $b_doc),
            ],
            outputs: &[$out],
            doc: $doc,
        }
    };
}

/// A unary-operator row. It emits a `UnaryOpUGen` selecting the operator at
/// the given `special_index`.
macro_rules! uop {
    ($kw:literal, $unit:literal, $ix:literal, $out:literal, $doc:literal $(,)?) => {
        UnitDesc {
            keyword: $kw,
            unit: $unit,
            special: Some(Special {
                unit: "UnaryOpUGen",
                index: $ix,
            }),
            rate: UnitRate::Any,
            inputs: &[sig("in", "input signal")],
            outputs: &[$out],
            doc: $doc,
        }
    };
}

/// A hybrid oscillator/filter frequency param.
const fn freq(default: f32, doc: &'static str) -> In {
    par("freq", default, 0.0, 20_000.0, " Hz", doc)
}

/// The wrapped plyphon units, grouped by family. Signatures follow the
/// plyphon crate the workspace pins, in SC-conventional arg order.
pub static UNITS: &[UnitDesc] = &[
    // Oscillators
    u(
        "~saw",
        "Saw",
        &[freq(220.0, "frequency. A wire drives it directly")],
        &["sawtooth signal"],
        "Band-limited sawtooth oscillator",
    ),
    u(
        "~pulse",
        "Pulse",
        &[
            freq(220.0, "frequency. A wire drives it directly"),
            par("width", 0.5, 0.0, 1.0, "", "pulse width duty cycle"),
        ],
        &["pulse signal"],
        "Band-limited pulse-wave oscillator with settable width",
    ),
    u(
        "~blip",
        "Blip",
        &[
            freq(220.0, "fundamental frequency"),
            par("numharm", 200.0, 1.0, 500.0, "", "number of harmonics"),
        ],
        &["impulse-train signal"],
        "Band-limited impulse oscillator (harmonic count)",
    ),
    u(
        "~varsaw",
        "VarSaw",
        &[
            freq(220.0, "frequency"),
            baked(0.0),
            par(
                "width",
                0.5,
                0.0,
                1.0,
                "",
                "duty cycle (0 saw, 0.5 triangle)",
            ),
        ],
        &["variable-duty saw signal"],
        "Variable-duty sawtooth/triangle oscillator",
    ),
    u(
        "~syncsaw",
        "SyncSaw",
        &[
            par(
                "syncfreq",
                220.0,
                0.0,
                20_000.0,
                " Hz",
                "sync (reset) frequency",
            ),
            par(
                "sawfreq",
                440.0,
                0.0,
                20_000.0,
                " Hz",
                "slave sawtooth frequency",
            ),
        ],
        &["hard-synced saw signal"],
        "Hard-sync sawtooth oscillator",
    ),
    u(
        "~impulse",
        "Impulse",
        &[freq(1.0, "impulse frequency"), baked(0.0)],
        &["single-sample impulse train"],
        "Impulse (single-sample click) oscillator",
    ),
    u(
        "~fsinosc",
        "FSinOsc",
        &[freq(220.0, "frequency"), baked(0.0)],
        &["sine signal"],
        "Fast fixed-frequency sine oscillator (undamped resonator)",
    ),
    u(
        "~sinosc",
        "SinOsc",
        &[
            freq(220.0, "frequency. A wire drives it for FM"),
            baked(0.0),
        ],
        &["sine signal"],
        "Sine oscillator (audio or control rate)",
    ),
    u(
        "~sinoscfb",
        "SinOscFB",
        &[
            freq(220.0, "frequency"),
            par("feedback", 0.0, 0.0, 3.14, "", "phase-feedback amount"),
        ],
        &["feedback-sine signal"],
        "Sine oscillator with phase feedback",
    ),
    u(
        "~formant",
        "Formant",
        &[
            par(
                "fundfreq",
                440.0,
                0.0,
                20_000.0,
                " Hz",
                "fundamental frequency",
            ),
            par(
                "formfreq",
                1760.0,
                0.0,
                20_000.0,
                " Hz",
                "formant frequency",
            ),
            par("bwfreq", 880.0, 0.0, 20_000.0, " Hz", "formant bandwidth"),
        ],
        &["formant signal"],
        "Formant oscillator",
    ),
    u(
        "~lfsaw",
        "LFSaw",
        &[freq(220.0, "frequency"), baked(0.0)],
        &["sawtooth signal"],
        "Non-band-limited sawtooth oscillator/LFO",
    ),
    u(
        "~lftri",
        "LFTri",
        &[freq(220.0, "frequency"), baked(0.0)],
        &["triangle signal"],
        "Non-band-limited triangle oscillator/LFO",
    ),
    u(
        "~lfpar",
        "LFPar",
        &[freq(220.0, "frequency"), baked(0.0)],
        &["parabolic signal"],
        "Parabolic (sine-like) oscillator/LFO",
    ),
    u(
        "~lfcub",
        "LFCub",
        &[freq(220.0, "frequency"), baked(0.0)],
        &["cubic-sine signal"],
        "Cubic-sine oscillator/LFO",
    ),
    u(
        "~lfpulse",
        "LFPulse",
        &[
            freq(220.0, "frequency"),
            baked(0.0),
            par("width", 0.5, 0.0, 1.0, "", "pulse width duty cycle"),
        ],
        &["unipolar pulse signal"],
        "Non-band-limited pulse oscillator/LFO (unipolar)",
    ),
    // Noise
    u(
        "~whitenoise",
        "WhiteNoise",
        &[],
        &["white noise"],
        "White noise (flat spectrum)",
    ),
    u(
        "~pinknoise",
        "PinkNoise",
        &[],
        &["pink noise"],
        "Pink noise (equal energy per octave)",
    ),
    u(
        "~brownnoise",
        "BrownNoise",
        &[],
        &["brown noise"],
        "Brown noise (random walk)",
    ),
    u(
        "~clipnoise",
        "ClipNoise",
        &[],
        &["clipped noise"],
        "Random values at +/-1",
    ),
    u(
        "~graynoise",
        "GrayNoise",
        &[],
        &["gray noise"],
        "Gray noise (random bit flips)",
    ),
    u(
        "~dust",
        "Dust",
        &[par(
            "density",
            20.0,
            0.0,
            10_000.0,
            " Hz",
            "average impulses per second",
        )],
        &["random positive impulses"],
        "Random positive impulses at an average density",
    ),
    u(
        "~dust2",
        "Dust2",
        &[par(
            "density",
            20.0,
            0.0,
            10_000.0,
            " Hz",
            "average impulses per second",
        )],
        &["random bipolar impulses"],
        "Random bipolar impulses at an average density",
    ),
    u(
        "~crackle",
        "Crackle",
        &[par("chaos", 1.5, 1.0, 2.0, "", "chaos parameter")],
        &["crackle noise"],
        "Chaotic noise generator",
    ),
    u(
        "~lfnoise0",
        "LFNoise0",
        &[freq(500.0, "value-change frequency")],
        &["stepped random signal"],
        "Step noise: random values at a frequency",
    ),
    u(
        "~lfnoise1",
        "LFNoise1",
        &[freq(500.0, "value-change frequency")],
        &["ramped random signal"],
        "Ramp noise: linearly interpolated random values",
    ),
    u(
        "~lfnoise2",
        "LFNoise2",
        &[freq(500.0, "value-change frequency")],
        &["curved random signal"],
        "Quadratic noise: smoothly interpolated random values",
    ),
    u(
        "~lfclipnoise",
        "LFClipNoise",
        &[freq(500.0, "value-change frequency")],
        &["random +/-1 steps"],
        "Clipped step noise: random +/-1 values at a frequency",
    ),
    // Filters
    u(
        "~lpf",
        "LPF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "cutoff frequency"),
        ],
        &["low-passed signal"],
        "2nd-order Butterworth low-pass filter",
    ),
    u(
        "~hpf",
        "HPF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "cutoff frequency"),
        ],
        &["high-passed signal"],
        "2nd-order Butterworth high-pass filter",
    ),
    u(
        "~bpf",
        "BPF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "center frequency"),
            par("bw", 1.0, 0.01, 10.0, "", "bandwidth / center frequency"),
        ],
        &["band-passed signal"],
        "2nd-order Butterworth band-pass filter",
    ),
    u(
        "~brf",
        "BRF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "center frequency"),
            par("bw", 1.0, 0.01, 10.0, "", "bandwidth / center frequency"),
        ],
        &["band-rejected signal"],
        "2nd-order Butterworth band-reject (notch) filter",
    ),
    u(
        "~rlpf",
        "RLPF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "cutoff frequency"),
            par(
                "rq",
                1.0,
                0.01,
                10.0,
                "",
                "reciprocal of Q (bandwidth / cutoff)",
            ),
        ],
        &["low-passed signal"],
        "Resonant low-pass filter",
    ),
    u(
        "~rhpf",
        "RHPF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "cutoff frequency"),
            par(
                "rq",
                1.0,
                0.01,
                10.0,
                "",
                "reciprocal of Q (bandwidth / cutoff)",
            ),
        ],
        &["high-passed signal"],
        "Resonant high-pass filter",
    ),
    u(
        "~resonz",
        "Resonz",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "resonant frequency"),
            par(
                "bwr",
                1.0,
                0.01,
                10.0,
                "",
                "bandwidth ratio (bandwidth / center)",
            ),
        ],
        &["resonant band-passed signal"],
        "Resonant band-pass filter (constant gain)",
    ),
    u(
        "~ringz",
        "Ringz",
        &[
            sig("in", "signal to ring"),
            freq(440.0, "resonant frequency"),
            par("decay", 1.0, 0.0, 60.0, " s", "ring decay time"),
        ],
        &["ringing signal"],
        "Ringing resonator (bell-like decaying resonance)",
    ),
    u(
        "~moogff",
        "MoogFF",
        &[
            sig("in", "signal to filter"),
            freq(440.0, "cutoff frequency"),
            par(
                "gain",
                2.0,
                0.0,
                4.0,
                "",
                "resonance gain (self-oscillates near 4)",
            ),
            baked(0.0),
        ],
        &["low-passed signal"],
        "Moog-style 4-pole ladder low-pass filter",
    ),
    u(
        "~onepole",
        "OnePole",
        &[
            sig("in", "signal to filter"),
            par(
                "coef",
                0.5,
                -1.0,
                1.0,
                "",
                "feedback coefficient (+lowpass, -highpass)",
            ),
        ],
        &["filtered signal"],
        "One-pole filter",
    ),
    u(
        "~onezero",
        "OneZero",
        &[
            sig("in", "signal to filter"),
            par(
                "coef",
                0.5,
                -1.0,
                1.0,
                "",
                "feedforward coefficient (+lowpass, -highpass)",
            ),
        ],
        &["filtered signal"],
        "One-zero filter",
    ),
    u(
        "~leakdc",
        "LeakDC",
        &[
            sig("in", "signal to de-offset"),
            par("coef", 0.995, 0.9, 1.0, "", "leak coefficient"),
        ],
        &["DC-blocked signal"],
        "DC-blocking leaky high-pass",
    ),
    u(
        "~slew",
        "Slew",
        &[
            sig("in", "signal to limit"),
            par(
                "up",
                1.0,
                0.0,
                10_000.0,
                "/s",
                "max upward slope per second",
            ),
            par(
                "dn",
                1.0,
                0.0,
                10_000.0,
                "/s",
                "max downward slope per second",
            ),
        ],
        &["slope-limited signal"],
        "Slew-rate limiter",
    ),
    u(
        "~lag",
        "Lag",
        &[
            sig("in", "signal to smooth"),
            par(
                "dur",
                0.1,
                0.0,
                10.0,
                " s",
                "smoothing duration (60 dB convergence)",
            ),
        ],
        &["smoothed signal"],
        "One-pole smoother over a duration",
    ),
    u(
        "~lag2",
        "Lag2",
        &[
            sig("in", "signal to smooth"),
            par("dur", 0.1, 0.0, 10.0, " s", "smoothing duration per stage"),
        ],
        &["smoothed signal"],
        "Twice-cascaded one-pole smoother",
    ),
    u(
        "~lag3",
        "Lag3",
        &[
            sig("in", "signal to smooth"),
            par("dur", 0.1, 0.0, 10.0, " s", "smoothing duration per stage"),
        ],
        &["smoothed signal"],
        "Thrice-cascaded one-pole smoother",
    ),
    u(
        "~mideq",
        "MidEQ",
        &[
            sig("in", "signal to equalize"),
            freq(440.0, "center frequency"),
            par("rq", 1.0, 0.01, 10.0, "", "reciprocal of Q"),
            par(
                "db",
                0.0,
                -24.0,
                24.0,
                " dB",
                "boost/cut at the center frequency",
            ),
        ],
        &["equalized signal"],
        "Parametric mid-band equalizer",
    ),
    u(
        "~formlet",
        "Formlet",
        &[
            sig("in", "excitation signal"),
            freq(440.0, "resonant frequency"),
            par("attack", 1.0, 0.0, 10.0, " s", "onset time"),
            par("decay", 1.0, 0.0, 10.0, " s", "decay time"),
        ],
        &["formant-impulse signal"],
        "FOF-like resonant filter (formant impulse response)",
    ),
    u(
        "~decay",
        "Decay",
        &[
            sig("in", "impulses to integrate"),
            par("decay", 1.0, 0.0, 60.0, " s", "60 dB decay time"),
        ],
        &["decay envelope signal"],
        "Exponential decay integrator (triggered envelopes from impulses)",
    ),
    u(
        "~decay2",
        "Decay2",
        &[
            sig("in", "impulses to integrate"),
            par("attack", 0.01, 0.0, 60.0, " s", "attack time"),
            par("decay", 1.0, 0.0, 60.0, " s", "60 dB decay time"),
        ],
        &["attack-decay envelope signal"],
        "Attack-decay integrator (smoothed impulse envelopes)",
    ),
    // Delays
    u(
        "~delayn",
        "DelayN",
        &[
            sig("in", "signal to delay"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
        ],
        &["delayed signal"],
        "Simple delay line (no interpolation)",
    ),
    u(
        "~delayl",
        "DelayL",
        &[
            sig("in", "signal to delay"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
        ],
        &["delayed signal"],
        "Simple delay line (linear interpolation)",
    ),
    u(
        "~delayc",
        "DelayC",
        &[
            sig("in", "signal to delay"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
        ],
        &["delayed signal"],
        "Simple delay line (cubic interpolation)",
    ),
    u(
        "~combn",
        "CombN",
        &[
            sig("in", "signal to comb-filter"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par(
                "decay",
                1.0,
                -60.0,
                60.0,
                " s",
                "60 dB feedback decay time (negative alternates sign)",
            ),
        ],
        &["comb-filtered signal"],
        "Comb (feedback) delay, no interpolation",
    ),
    u(
        "~combl",
        "CombL",
        &[
            sig("in", "signal to comb-filter"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par(
                "decay",
                1.0,
                -60.0,
                60.0,
                " s",
                "60 dB feedback decay time (negative alternates sign)",
            ),
        ],
        &["comb-filtered signal"],
        "Comb (feedback) delay, linear interpolation",
    ),
    u(
        "~combc",
        "CombC",
        &[
            sig("in", "signal to comb-filter"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par(
                "decay",
                1.0,
                -60.0,
                60.0,
                " s",
                "60 dB feedback decay time (negative alternates sign)",
            ),
        ],
        &["comb-filtered signal"],
        "Comb (feedback) delay, cubic interpolation",
    ),
    u(
        "~allpassn",
        "AllpassN",
        &[
            sig("in", "signal to diffuse"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par("decay", 1.0, -60.0, 60.0, " s", "60 dB feedback decay time"),
        ],
        &["all-passed signal"],
        "All-pass (phase-dispersing feedback) delay, no interpolation",
    ),
    u(
        "~allpassl",
        "AllpassL",
        &[
            sig("in", "signal to diffuse"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par("decay", 1.0, -60.0, 60.0, " s", "60 dB feedback decay time"),
        ],
        &["all-passed signal"],
        "All-pass (phase-dispersing feedback) delay, linear interpolation",
    ),
    u(
        "~allpassc",
        "AllpassC",
        &[
            sig("in", "signal to diffuse"),
            init(
                "maxdelay",
                0.2,
                "max delay time. Sizes the delay line and re-derives",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par("decay", 1.0, -60.0, 60.0, " s", "60 dB feedback decay time"),
        ],
        &["all-passed signal"],
        "All-pass (phase-dispersing feedback) delay, cubic interpolation",
    ),
    // Lines
    u(
        "~line",
        "Line",
        &[
            init("start", 0.0, "start value. Latched at spawn and re-derives"),
            init("end", 1.0, "end value. Latched at spawn and re-derives"),
            init(
                "dur",
                1.0,
                "ramp duration in seconds. Latched at spawn and re-derives",
            ),
            baked(0.0),
        ],
        &["linear ramp signal"],
        "Linear ramp from start to end (values latch when the synth spawns)",
    ),
    u(
        "~xline",
        "XLine",
        &[
            init(
                "start",
                1.0,
                "start value, nonzero. Latched at spawn and re-derives",
            ),
            init(
                "end",
                2.0,
                "end value, same sign. Latched at spawn and re-derives",
            ),
            init(
                "dur",
                1.0,
                "ramp duration in seconds. Latched at spawn and re-derives",
            ),
            baked(0.0),
        ],
        &["exponential ramp signal"],
        "Exponential ramp from start to end (values latch when the synth spawns)",
    ),
    // Dynamics
    u(
        "~limiter",
        "Limiter",
        &[
            sig("in", "signal to limit"),
            par("level", 1.0, 0.0, 2.0, "", "peak output amplitude"),
            init(
                "dur",
                0.01,
                "look-ahead time. Sizes the buffer and re-derives",
            ),
        ],
        &["limited signal"],
        "Look-ahead peak limiter",
    ),
    u(
        "~normalizer",
        "Normalizer",
        &[
            sig("in", "signal to normalize"),
            par("level", 1.0, 0.0, 2.0, "", "target peak amplitude"),
            init(
                "dur",
                0.01,
                "look-ahead time. Sizes the buffer and re-derives",
            ),
        ],
        &["normalized signal"],
        "Look-ahead amplitude normalizer (flattens dynamics)",
    ),
    u(
        "~amplitude",
        "Amplitude",
        &[
            sig("in", "signal to follow"),
            par("attack", 0.01, 0.0, 10.0, " s", "follower attack time"),
            par("release", 0.01, 0.0, 10.0, " s", "follower release time"),
        ],
        &["amplitude envelope"],
        "Amplitude (envelope) follower",
    ),
    // Pan and mix
    u(
        "~pan2",
        "Pan2",
        &[
            sig("in", "signal to pan"),
            par(
                "pos",
                0.0,
                -1.0,
                1.0,
                "",
                "pan position (-1 left .. 1 right)",
            ),
            par("level", 1.0, 0.0, 2.0, "", "output level"),
        ],
        &["left channel", "right channel"],
        "Equal-power stereo panner",
    ),
    u(
        "~linpan2",
        "LinPan2",
        &[
            sig("in", "signal to pan"),
            par(
                "pos",
                0.0,
                -1.0,
                1.0,
                "",
                "pan position (-1 left .. 1 right)",
            ),
            par("level", 1.0, 0.0, 2.0, "", "output level"),
        ],
        &["left channel", "right channel"],
        "Linear-crossfade stereo panner",
    ),
    u(
        "~balance2",
        "Balance2",
        &[
            sig("left", "left input signal"),
            sig("right", "right input signal"),
            par(
                "pos",
                0.0,
                -1.0,
                1.0,
                "",
                "balance position (-1 left .. 1 right)",
            ),
            par("level", 1.0, 0.0, 2.0, "", "output level"),
        ],
        &["left channel", "right channel"],
        "Stereo balance (attenuates the opposite side)",
    ),
    u(
        "~xfade2",
        "XFade2",
        &[
            sig("a", "first input signal"),
            sig("b", "second input signal"),
            par(
                "pan",
                0.0,
                -1.0,
                1.0,
                "",
                "crossfade position (-1 = a, 1 = b)",
            ),
            par("level", 1.0, 0.0, 2.0, "", "output level"),
        ],
        &["crossfaded signal"],
        "Equal-power two-signal crossfade",
    ),
    u(
        "~rotate2",
        "Rotate2",
        &[
            sig("x", "first input signal"),
            sig("y", "second input signal"),
            par(
                "pos",
                0.0,
                -1.0,
                1.0,
                "",
                "rotation position (2 = full circle)",
            ),
        ],
        &["rotated x", "rotated y"],
        "Rotate a two-channel sound field",
    ),
    // Math and range
    u(
        "~muladd",
        "MulAdd",
        &[
            sig("in", "signal to scale and offset"),
            par("mul", 1.0, -10_000.0, 10_000.0, "", "multiplier"),
            par("add", 0.0, -10_000.0, 10_000.0, "", "offset"),
        ],
        &["scaled signal"],
        "in * mul + add",
    ),
    u(
        "~linexp",
        "LinExp",
        &[
            sig("in", "signal to map"),
            par("srclo", 0.0, -10_000.0, 10_000.0, "", "source range low"),
            par("srchi", 1.0, -10_000.0, 10_000.0, "", "source range high"),
            par(
                "dstlo",
                1.0,
                -10_000.0,
                10_000.0,
                "",
                "destination range low (nonzero)",
            ),
            par(
                "dsthi",
                2.0,
                -10_000.0,
                10_000.0,
                "",
                "destination range high (same sign)",
            ),
        ],
        &["mapped signal"],
        "Map a linear input range onto an exponential output range",
    ),
    u(
        "~clip",
        "Clip",
        &[
            sig("in", "signal to clip"),
            par("lo", -1.0, -10_000.0, 10_000.0, "", "lower bound"),
            par("hi", 1.0, -10_000.0, 10_000.0, "", "upper bound"),
        ],
        &["clipped signal"],
        "Clip a signal to [lo, hi]",
    ),
    u(
        "~wrap",
        "Wrap",
        &[
            sig("in", "signal to wrap"),
            par("lo", -1.0, -10_000.0, 10_000.0, "", "lower bound"),
            par("hi", 1.0, -10_000.0, 10_000.0, "", "upper bound"),
        ],
        &["wrapped signal"],
        "Wrap a signal into [lo, hi]",
    ),
    u(
        "~fold",
        "Fold",
        &[
            sig("in", "signal to fold"),
            par("lo", -1.0, -10_000.0, 10_000.0, "", "lower bound"),
            par("hi", 1.0, -10_000.0, 10_000.0, "", "upper bound"),
        ],
        &["folded signal"],
        "Fold (mirror) a signal into [lo, hi]",
    ),
    // Triggers. A trigger is a rising edge, a sample above 0 after one at or
    // below 0.
    u(
        "~trig",
        "Trig",
        &[
            sig("trig", "trigger signal"),
            par("dur", 0.1, 0.0, 60.0, " s", "hold time after each trigger"),
        ],
        &["the trigger's value held for dur, else 0"],
        "Hold a trigger's value for a duration after each rising edge",
    ),
    u(
        "~trig1",
        "Trig1",
        &[
            sig("trig", "trigger signal"),
            par("dur", 0.1, 0.0, 60.0, " s", "hold time after each trigger"),
        ],
        &["1 for dur after a trigger, else 0"],
        "Output 1 for a duration after each rising edge",
    ),
    u(
        "~tdelay",
        "TDelay",
        &[
            sig("trig", "trigger signal"),
            par("dur", 0.1, 0.0, 60.0, " s", "delay time"),
        ],
        &["delayed single-sample triggers"],
        "Delay each rising edge by a duration",
    ),
    u(
        "~latch",
        "Latch",
        &[
            sig("in", "signal to sample"),
            sig("trig", "sample on each rising edge"),
        ],
        &["sampled and held signal"],
        "Sample and hold: sample the input on each trigger",
    ),
    u(
        "~gate",
        "Gate",
        &[
            sig("in", "signal to gate"),
            sig("trig", "pass the input while above 0"),
        ],
        &["gated signal"],
        "Pass the input while the gate is open, else hold the last value",
    ),
    u(
        "~toggleff",
        "ToggleFF",
        &[sig("trig", "toggle on each rising edge")],
        &["0 or 1"],
        "Toggle flip-flop: flip between 0 and 1 on each trigger",
    ),
    u(
        "~setresetff",
        "SetResetFF",
        &[
            sig("trig", "set to 1 on a rising edge"),
            sig("reset", "reset to 0 on a rising edge"),
        ],
        &["0 or 1"],
        "Set-reset flip-flop (reset wins when both fire)",
    ),
    u(
        "~schmidt",
        "Schmidt",
        &[
            sig("in", "signal to threshold"),
            par("lo", 0.0, -10_000.0, 10_000.0, "", "fall below to output 0"),
            par("hi", 1.0, -10_000.0, 10_000.0, "", "rise above to output 1"),
        ],
        &["0 or 1 with hysteresis"],
        "Schmitt trigger: 1 once above hi, 0 once below lo",
    ),
    // Timing. Counters and ramps driven by triggers.
    u(
        "~pulsecount",
        "PulseCount",
        &[
            sig("trig", "count each rising edge"),
            sig("reset", "zero the count on a rising edge"),
        ],
        &["trigger count"],
        "Count rising edges",
    ),
    u(
        "~pulsedivider",
        "PulseDivider",
        &[
            sig("trig", "trigger signal"),
            par("div", 2.0, 1.0, 1000.0, "", "pass every div-th trigger"),
            par("start", 0.0, -1000.0, 1000.0, "", "starting count"),
        ],
        &["1 on every div-th trigger, else 0"],
        "Pass every n-th rising edge",
    ),
    u(
        "~stepper",
        "Stepper",
        &[
            sig("trig", "step on each rising edge"),
            sig("reset", "jump to resetval on a rising edge"),
            par("min", 0.0, -10_000.0, 10_000.0, "", "lowest count"),
            par("max", 7.0, -10_000.0, 10_000.0, "", "highest count"),
            par("step", 1.0, -10_000.0, 10_000.0, "", "step per trigger"),
            par(
                "resetval",
                0.0,
                -10_000.0,
                10_000.0,
                "",
                "count after a reset",
            ),
        ],
        &["stepped integer count"],
        "Counter stepping by step and wrapping within [min, max]",
    ),
    u(
        "~zerocrossing",
        "ZeroCrossing",
        &[sig("in", "signal to measure")],
        &["estimated frequency in Hz"],
        "Estimate the fundamental frequency from the zero-crossing period",
    ),
    u(
        "~timer",
        "Timer",
        &[sig("trig", "trigger signal")],
        &["seconds since the previous trigger"],
        "Time in seconds between successive rising edges",
    ),
    u(
        "~sweep",
        "Sweep",
        &[
            sig("trig", "restart the ramp on a rising edge"),
            par(
                "speed",
                1.0,
                -10_000.0,
                10_000.0,
                "/s",
                "ramp rate per second",
            ),
        ],
        &["linear ramp"],
        "Linear ramp climbing at a rate per second, restarted by a trigger",
    ),
    u(
        "~phasor",
        "Phasor",
        &[
            sig("trig", "jump to resetpos on a rising edge"),
            par(
                "speed",
                1.0,
                -10_000.0,
                10_000.0,
                "",
                "increment per sample (or per block at kr)",
            ),
            par("start", 0.0, -10_000.0, 10_000.0, "", "wrap range start"),
            par("end", 1.0, -10_000.0, 10_000.0, "", "wrap range end"),
            par(
                "resetpos",
                0.0,
                -10_000.0,
                10_000.0,
                "",
                "position after a trigger",
            ),
        ],
        &["wrapping ramp"],
        "Ramp advancing by an increment per sample and wrapping within [start, end)",
    ),
    // Measurement. Running statistics of a signal.
    u(
        "~peak",
        "Peak",
        &[
            sig("in", "signal to measure"),
            sig("trig", "reset the peak on a rising edge"),
        ],
        &["running peak of |in|"],
        "Running peak absolute value, reset by a trigger",
    ),
    u(
        "~runningmin",
        "RunningMin",
        &[
            sig("in", "signal to measure"),
            sig("trig", "reset the minimum on a rising edge"),
        ],
        &["running minimum"],
        "Running minimum, reset by a trigger",
    ),
    u(
        "~runningmax",
        "RunningMax",
        &[
            sig("in", "signal to measure"),
            sig("trig", "reset the maximum on a rising edge"),
        ],
        &["running maximum"],
        "Running maximum, reset by a trigger",
    ),
    u(
        "~peakfollower",
        "PeakFollower",
        &[
            sig("in", "signal to follow"),
            par("decay", 0.999, 0.0, 1.0, "", "per-sample decay factor"),
        ],
        &["amplitude envelope"],
        "Envelope follower with an instant attack and an exponential release",
    ),
    u(
        "~mostchange",
        "MostChange",
        &[sig("a", "first signal"), sig("b", "second signal")],
        &["whichever input changed more"],
        "Pass whichever of two signals changed the most since the last sample",
    ),
    u(
        "~leastchange",
        "LeastChange",
        &[sig("a", "first signal"), sig("b", "second signal")],
        &["whichever input changed less"],
        "Pass whichever of two signals changed the least since the last sample",
    ),
    u(
        "~lastvalue",
        "LastValue",
        &[
            sig("in", "signal to quantise"),
            par(
                "diff",
                0.01,
                0.0,
                10_000.0,
                "",
                "change needed to accept a new value",
            ),
        ],
        &["held value"],
        "Sample and hold that steps only once the input moves by diff",
    ),
    // Rate conversion. The converters run at their target rate only.
    u(
        "~dc",
        "DC",
        &[par(
            "value",
            0.0,
            -10_000.0,
            10_000.0,
            "",
            "the constant value",
        )],
        &["constant signal"],
        "A constant signal at audio or control rate",
    ),
    ar_only(u(
        "~k2a",
        "K2A",
        &[sig("in", "control-rate signal to lift")],
        &["audio-rate signal"],
        "Control to audio rate, ramping linearly across each block (ar only)",
    )),
    kr_only(u(
        "~a2k",
        "A2K",
        &[sig("in", "audio-rate signal to sample")],
        &["control-rate signal"],
        "Audio to control rate, taking each block's first sample (kr only)",
    )),
    ar_only(u(
        "~t2a",
        "T2A",
        &[
            sig("in", "control-rate trigger"),
            par(
                "offset",
                0.0,
                0.0,
                64.0,
                "",
                "sample offset within the block the trigger lands at",
            ),
        ],
        &["audio-rate trigger"],
        "Control-rate trigger to a sample-accurate audio trigger (ar only)",
    )),
    kr_only(u(
        "~t2k",
        "T2K",
        &[sig("in", "audio-rate trigger")],
        &["control-rate trigger"],
        "Audio-rate trigger to control rate, keeping the block's maximum so no \
         trigger is missed (kr only)",
    )),
    // Operators. One row per operator in plyphon's dispatch tables, which
    // follow SC's operator indices. Defaults for `b` are 1 for multiplicative
    // operators and 0 otherwise.
    bop!(
        "~add",
        "Add",
        0,
        0.0,
        "addend",
        "a + b",
        "Add the two inputs"
    ),
    bop!(
        "~sub",
        "Sub",
        1,
        0.0,
        "subtrahend",
        "a - b",
        "Subtract `b` from `a`",
    ),
    bop!(
        "~mul",
        "Mul",
        2,
        1.0,
        "multiplier",
        "a * b",
        "Multiply the two inputs (ring modulation when both are signals, \
         a gain otherwise)",
    ),
    bop!(
        "~idiv",
        "IDiv",
        3,
        1.0,
        "divisor",
        "floor(a / b)",
        "Integer division: divide and round down",
    ),
    bop!(
        "~div",
        "Div",
        4,
        1.0,
        "divisor",
        "a / b",
        "Divide `a` by `b`"
    ),
    bop!(
        "~mod",
        "Mod",
        5,
        1.0,
        "divisor",
        "a mod b",
        "Floating-point modulo (SC `mod` semantics)",
    ),
    bop!(
        "~eq",
        "Eq",
        6,
        0.0,
        "comparand",
        "1 when a == b, else 0",
        "Equality comparator gate",
    ),
    bop!(
        "~ne",
        "Ne",
        7,
        0.0,
        "comparand",
        "1 when a != b, else 0",
        "Inequality comparator gate",
    ),
    bop!(
        "~lt",
        "Lt",
        8,
        0.0,
        "threshold",
        "1 when a < b, else 0",
        "Less-than comparator gate",
    ),
    bop!(
        "~gt",
        "Gt",
        9,
        0.0,
        "threshold",
        "1 when a > b, else 0",
        "Greater-than comparator gate",
    ),
    bop!(
        "~le",
        "Le",
        10,
        0.0,
        "threshold",
        "1 when a <= b, else 0",
        "Less-than-or-equal comparator gate",
    ),
    bop!(
        "~ge",
        "Ge",
        11,
        0.0,
        "threshold",
        "1 when a >= b, else 0",
        "Greater-than-or-equal comparator gate",
    ),
    bop!(
        "~min",
        "Min",
        12,
        0.0,
        "ceiling",
        "min(a, b)",
        "Minimum of the two inputs",
    ),
    bop!(
        "~max",
        "Max",
        13,
        0.0,
        "floor",
        "max(a, b)",
        "Maximum of the two inputs",
    ),
    bop!(
        "~bitand",
        "BitAnd",
        14,
        0.0,
        "operand",
        "a AND b",
        "Bitwise AND of the inputs truncated to integers",
    ),
    bop!(
        "~bitor",
        "BitOr",
        15,
        0.0,
        "operand",
        "a OR b",
        "Bitwise OR of the inputs truncated to integers",
    ),
    bop!(
        "~bitxor",
        "BitXor",
        16,
        0.0,
        "operand",
        "a XOR b",
        "Bitwise XOR of the inputs truncated to integers",
    ),
    bop!(
        "~lcm",
        "Lcm",
        17,
        1.0,
        "operand",
        "lcm(a, b)",
        "Least common multiple (integer semantics)",
    ),
    bop!(
        "~gcd",
        "Gcd",
        18,
        1.0,
        "operand",
        "gcd(a, b)",
        "Greatest common divisor (integer semantics)",
    ),
    bop!(
        "~round",
        "Round",
        19,
        1.0,
        "quantum",
        "a rounded to the nearest multiple of b",
        "Round to a multiple of `b`",
    ),
    bop!(
        "~roundup",
        "RoundUp",
        20,
        1.0,
        "quantum",
        "a rounded up to a multiple of b",
        "Round up to a multiple of `b`",
    ),
    bop!(
        "~trunc",
        "Trunc",
        21,
        1.0,
        "quantum",
        "a truncated to a multiple of b",
        "Truncate to a multiple of `b`",
    ),
    bop!(
        "~atan2",
        "Atan2",
        22,
        1.0,
        "x coordinate",
        "atan2(a, b) in radians",
        "Arctangent of `a / b` using both signs (with b at its default 1, \
         plain atan of `a`)",
    ),
    bop!(
        "~hypot",
        "Hypot",
        23,
        0.0,
        "operand",
        "sqrt(a^2 + b^2)",
        "Hypotenuse (distance) of the two inputs",
    ),
    bop!(
        "~hypotx",
        "Hypotx",
        24,
        0.0,
        "operand",
        "approximate hypotenuse",
        "Cheap approximate hypotenuse (SC `hypotApx`)",
    ),
    bop!(
        "~pow",
        "Pow",
        25,
        1.0,
        "exponent",
        "a ^ b",
        "Raise `a` to the power `b` (SC sign-preserving pow)",
    ),
    bop!(
        "~shiftleft",
        "ShiftLeft",
        26,
        0.0,
        "bit count",
        "a << b",
        "Bitwise left shift of the inputs truncated to integers",
    ),
    bop!(
        "~shiftright",
        "ShiftRight",
        27,
        0.0,
        "bit count",
        "a >> b",
        "Bitwise right shift of the inputs truncated to integers",
    ),
    bop!(
        "~ring1",
        "Ring1",
        30,
        0.0,
        "modulator",
        "a * b + a",
        "Ring modulation plus the carrier",
    ),
    bop!(
        "~ring2",
        "Ring2",
        31,
        0.0,
        "modulator",
        "a * b + a + b",
        "Ring modulation plus both inputs",
    ),
    bop!(
        "~ring3",
        "Ring3",
        32,
        0.0,
        "modulator",
        "a * a * b",
        "Ring modulation variant `a^2 * b`",
    ),
    bop!(
        "~ring4",
        "Ring4",
        33,
        0.0,
        "modulator",
        "a^2 * b - a * b^2",
        "Ring modulation variant",
    ),
    bop!(
        "~difsqr",
        "DifSqr",
        34,
        0.0,
        "operand",
        "a^2 - b^2",
        "Difference of squares",
    ),
    bop!(
        "~sumsqr",
        "SumSqr",
        35,
        0.0,
        "operand",
        "a^2 + b^2",
        "Sum of squares",
    ),
    bop!(
        "~sqrsum",
        "SqrSum",
        36,
        0.0,
        "operand",
        "(a + b)^2",
        "Square of the sum",
    ),
    bop!(
        "~sqrdif",
        "SqrDif",
        37,
        0.0,
        "operand",
        "(a - b)^2",
        "Square of the difference",
    ),
    bop!(
        "~absdif",
        "AbsDif",
        38,
        0.0,
        "operand",
        "|a - b|",
        "Absolute difference",
    ),
    bop!(
        "~thresh",
        "Thresh",
        39,
        0.0,
        "threshold",
        "0 when a < b, else a",
        "Thresholding gate: silence `a` below `b`",
    ),
    bop!(
        "~amclip",
        "AmClip",
        40,
        1.0,
        "gain",
        "a * b when b > 0, else 0",
        "Two-quadrant amplitude modulation",
    ),
    bop!(
        "~scaleneg",
        "ScaleNeg",
        41,
        1.0,
        "scale",
        "a scaled by b when a < 0, else a",
        "Scale only the negative half of the signal",
    ),
    bop!(
        "~clip2",
        "Clip2",
        42,
        1.0,
        "limit",
        "a clipped into [-b, b]",
        "Bilateral hard clip",
    ),
    bop!(
        "~excess",
        "Excess",
        43,
        1.0,
        "limit",
        "a - clip2(a, b)",
        "The residual removed by clipping",
    ),
    bop!(
        "~fold2",
        "Fold2",
        44,
        1.0,
        "limit",
        "a folded into [-b, b]",
        "Bilateral fold-back distortion",
    ),
    bop!(
        "~wrap2",
        "Wrap2",
        45,
        1.0,
        "limit",
        "a wrapped into [-b, b]",
        "Bilateral wrap-around",
    ),
    bop!(
        "~firstarg",
        "FirstArg",
        46,
        0.0,
        "ignored operand",
        "a",
        "Pass `a` through, ignoring `b` (forces a dependency on `b`)",
    ),
    uop!("~neg", "Neg", 0, "-in", "Negate the input"),
    uop!(
        "~not",
        "Not",
        1,
        "1 when in == 0, else 0",
        "Logical NOT gate"
    ),
    uop!(
        "~bitnot",
        "BitNot",
        4,
        "NOT in",
        "Bitwise NOT of the input truncated to an integer",
    ),
    uop!(
        "~abs",
        "Abs",
        5,
        "|in|",
        "Absolute value (full-wave rectify)"
    ),
    uop!(
        "~ceil",
        "Ceil",
        8,
        "in rounded up",
        "Round up to an integer"
    ),
    uop!(
        "~floor",
        "Floor",
        9,
        "in rounded down",
        "Round down to an integer",
    ),
    uop!("~frac", "Frac", 10, "in - floor(in)", "Fractional part"),
    uop!("~sign", "Sign", 11, "-1, 0 or 1", "Sign of the input"),
    uop!("~squared", "Squared", 12, "in^2", "Square the input"),
    uop!("~cubed", "Cubed", 13, "in^3", "Cube the input"),
    uop!(
        "~sqrt",
        "Sqrt",
        14,
        "sqrt(in), sign-preserving",
        "Square root (negative inputs mirror: -sqrt(-in))",
    ),
    uop!("~exp", "Exp", 15, "e^in", "Natural exponential"),
    uop!("~recip", "Recip", 16, "1 / in", "Reciprocal"),
    uop!(
        "~midicps",
        "MidiCps",
        17,
        "frequency in Hz",
        "MIDI note number to cycles per second",
    ),
    uop!(
        "~cpsmidi",
        "CpsMidi",
        18,
        "MIDI note number",
        "Cycles per second to MIDI note number",
    ),
    uop!(
        "~midiratio",
        "MidiRatio",
        19,
        "frequency ratio",
        "MIDI interval in semitones to frequency ratio",
    ),
    uop!(
        "~ratiomidi",
        "RatioMidi",
        20,
        "interval in semitones",
        "Frequency ratio to MIDI interval in semitones",
    ),
    uop!(
        "~dbamp",
        "DbAmp",
        21,
        "linear amplitude",
        "Decibels to linear amplitude",
    ),
    uop!(
        "~ampdb",
        "AmpDb",
        22,
        "decibels",
        "Linear amplitude to decibels",
    ),
    uop!(
        "~octcps",
        "OctCps",
        23,
        "frequency in Hz",
        "Decimal octaves to cycles per second",
    ),
    uop!(
        "~cpsoct",
        "CpsOct",
        24,
        "decimal octaves",
        "Cycles per second to decimal octaves",
    ),
    uop!("~log", "Log", 25, "ln(in)", "Natural logarithm"),
    uop!("~log2", "Log2", 26, "log2(in)", "Base-2 logarithm"),
    uop!(
        "~log10",
        "Log10",
        27,
        "log10(|in|)",
        "Base-10 logarithm of the absolute value",
    ),
    uop!("~sin", "Sin", 28, "sin(in)", "Sine (radians)"),
    uop!("~cos", "Cos", 29, "cos(in)", "Cosine (radians)"),
    uop!("~tan", "Tan", 30, "tan(in)", "Tangent (radians)"),
    uop!("~asin", "Asin", 31, "asin(in)", "Arcsine"),
    uop!("~acos", "Acos", 32, "acos(in)", "Arccosine"),
    uop!("~atan", "Atan", 33, "atan(in)", "Arctangent"),
    uop!("~sinh", "SinH", 34, "sinh(in)", "Hyperbolic sine"),
    uop!("~cosh", "CosH", 35, "cosh(in)", "Hyperbolic cosine"),
    uop!(
        "~tanh",
        "TanH",
        36,
        "tanh(in)",
        "Hyperbolic tangent (soft saturation)",
    ),
    uop!(
        "~distort",
        "Distort",
        42,
        "in / (1 + |in|)",
        "Nonlinear distortion",
    ),
    uop!(
        "~softclip",
        "SoftClip",
        43,
        "softly clipped in",
        "Soft clip: linear below +/-0.5, curved above",
    ),
    uop!(
        "~silence",
        "Silence",
        46,
        "0",
        "Silence, ignoring the input"
    ),
    uop!(
        "~thru",
        "Thru",
        47,
        "in",
        "Pass the input through unchanged"
    ),
    uop!(
        "~rectwindow",
        "RectWindow",
        48,
        "1 inside [0, 1], else 0",
        "Rectangular window over input phase 0..1",
    ),
    uop!(
        "~hanwindow",
        "HanWindow",
        49,
        "Hann window of in",
        "Hann window over input phase 0..1",
    ),
    uop!(
        "~welchwindow",
        "WelchWindow",
        50,
        "Welch window of in",
        "Welch window over input phase 0..1",
    ),
    uop!(
        "~triwindow",
        "TriWindow",
        51,
        "triangle window of in",
        "Triangle window over input phase 0..1",
    ),
    uop!(
        "~opramp",
        "OpRamp",
        52,
        "in clamped into [0, 1]",
        "Ramp shaping: clamp to the unit range",
    ),
    uop!(
        "~scurve",
        "SCurve",
        53,
        "smoothstep of in",
        "S-curve (smoothstep) shaping over 0..1",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_unit_is_registered_in_plyphon() {
        let registry = plyphon::UnitRegistry::with_builtins();
        let names: HashSet<&str> = registry.names().collect();
        for desc in UNITS {
            assert!(
                names.contains(desc.emitted_unit()),
                "unit `{}` is not registered in plyphon",
                desc.emitted_unit()
            );
        }
    }

    #[test]
    fn operator_rows_are_well_formed() {
        let registry = plyphon::UnitRegistry::with_builtins();
        let names: HashSet<&str> = registry.names().collect();
        for desc in UNITS {
            let Some(special) = desc.special else {
                continue;
            };
            // A pseudo identity must never shadow a real registry unit, so a
            // future plain row can always wrap that unit under its own name.
            assert!(
                !names.contains(desc.unit),
                "operator row identity `{}` shadows a plyphon unit",
                desc.unit
            );
            // plyphon's op ctors hard-reject any other arity.
            let n_inputs = match special.unit {
                "BinaryOpUGen" => 2,
                "UnaryOpUGen" => 1,
                other => panic!("{}: unexpected operator unit `{other}`", desc.unit),
            };
            assert_eq!(desc.inputs.len(), n_inputs, "{}: input arity", desc.unit);
            assert_eq!(desc.n_sockets(), n_inputs, "{}: socket arity", desc.unit);
            assert_eq!(desc.outputs.len(), 1, "{}: output arity", desc.unit);
        }
    }

    #[test]
    fn keywords_are_unique_tilde_lowercase() {
        let mut seen = HashSet::new();
        for desc in UNITS {
            assert!(
                seen.insert(desc.keyword),
                "duplicate keyword {}",
                desc.keyword
            );
            assert!(
                desc.keyword.starts_with('~'),
                "keyword {} must start with `~`",
                desc.keyword
            );
            assert!(
                desc.keyword[1..]
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "keyword {} must be lowercase ascii",
                desc.keyword
            );
        }
    }

    #[test]
    fn unit_names_are_unique() {
        let mut seen = HashSet::new();
        for desc in UNITS {
            assert!(seen.insert(desc.unit), "duplicate unit {}", desc.unit);
        }
    }

    #[test]
    fn rows_are_well_formed() {
        for desc in UNITS {
            assert!(
                !desc.outputs.is_empty(),
                "{}: a unit node needs at least one output",
                desc.unit
            );
            let mut names = HashSet::new();
            for input in desc.inputs {
                let Some(name) = input.name() else { continue };
                assert!(
                    names.insert(name),
                    "{}: duplicate input name {name}",
                    desc.unit
                );
                assert!(
                    name.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                    "{}: input name {name} must be lowercase ascii",
                    desc.unit
                );
                if let In::Param {
                    default, min, max, ..
                } = input
                {
                    assert!(
                        min <= default && default <= max,
                        "{}: {name} default {default} outside [{min}, {max}]",
                        desc.unit
                    );
                }
            }
        }
    }

    #[test]
    fn lookups_agree() {
        for desc in UNITS {
            assert!(std::ptr::eq(unit_desc(desc.unit).unwrap(), desc));
            assert!(std::ptr::eq(
                unit_desc_by_keyword(desc.keyword).unwrap(),
                desc
            ));
        }
        assert!(unit_desc("NoSuchUnit").is_none());
        assert!(unit_desc_by_keyword("~nosuchunit").is_none());
    }

    /// The fixed-rate rows are exactly the units plyphon misbehaves with at
    /// the other rate. Extend the list when adding one.
    #[test]
    fn fixed_rate_rows_are_the_expected_set() {
        use crate::dsp::NodeRate::{Audio, Control};
        let fixed: Vec<(&str, NodeRate)> = UNITS
            .iter()
            .filter_map(|d| match d.rate {
                UnitRate::Fixed(rate) => Some((d.unit, rate)),
                UnitRate::Any => None,
            })
            .collect();
        let expected = [
            ("K2A", Audio),
            ("A2K", Control),
            ("T2A", Audio),
            ("T2K", Control),
        ];
        assert_eq!(fixed, expected);
        for (unit, rate) in expected {
            assert_eq!(unit_desc(unit).unwrap().default_rate(), rate);
        }
        assert_eq!(unit_desc("SinOsc").unwrap().default_rate(), Audio);
    }

    /// An input named `rate` would shadow the ugen-rate inspector row and the
    /// `#:rate` sugar keyword.
    #[test]
    fn no_input_is_named_rate() {
        for desc in UNITS {
            assert!(
                desc.inputs.iter().all(|i| i.name() != Some("rate")),
                "{}: an input is named `rate`",
                desc.unit
            );
        }
    }
}
