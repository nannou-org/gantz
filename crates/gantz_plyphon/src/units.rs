//! The descriptor table behind [`UnitNode`](crate::UnitNode): one
//! [`UnitDesc`] row per wrapped plyphon unit generator.
//!
//! plyphon's [`Unit`](plyphon::Unit)/[`UnitDef`](plyphon::UnitDef) traits
//! expose no metadata - a unit's arity, input roles and defaults live only in
//! its docs - so this table is where gantz declares each wrapped unit's
//! signature: its palette/sugar keyword, its inputs in plyphon order (and how
//! each is fed - see [`In`]) and its outputs. Adding a plyphon unit as a gantz
//! node is one new row.
//!
//! Excluded for now: buffer-reading units, variable-arity units (`EnvGen`,
//! `Klang`), demand-rate, FFT/PV, IO/routing (covered by the bespoke nodes)
//! and the operator-selector units (`BinaryOpUGen`/`UnaryOpUGen`).

/// How one plyphon input of a wrapped unit is fed.
///
/// `Signal` and `Param` entries are *sockets* (dsp input ports, in entry
/// order); `Baked` and `Init` entries are socket-less constants. Entry order
/// matches the unit's plyphon input order.
#[derive(Clone, Copy, Debug)]
pub enum In {
    /// A pure dsp input: a socket carrying a signal, mono silence when
    /// unconnected.
    Signal {
        /// The input's name (socket docs).
        name: &'static str,
        /// The socket's doc line.
        doc: &'static str,
    },
    /// A *hybrid* input (see [`NodeDsp::n_dsp_inputs`](crate::NodeDsp)): a
    /// socket whose connected signal drives the input directly, falling back
    /// to a settable control param otherwise. The param's *value* lives in the
    /// node's keyed VM state (see [`param`](crate::param)); its smoothing lag
    /// lives in the node weight.
    Param {
        /// The param's name: its VM-state key, inspector label and sugar stem.
        name: &'static str,
        /// The value a fresh node starts at.
        default: f32,
        /// The inspector's drag range minimum.
        min: f32,
        /// The inspector's drag range maximum.
        max: f32,
        /// The inspector's unit suffix (e.g. `" Hz"`), possibly empty.
        suffix: &'static str,
        /// The socket/param doc line.
        doc: &'static str,
    },
    /// A fixed constant with no socket (e.g. an initial phase the node does
    /// not expose).
    Baked(f32),
    /// An *init-only* structural value with no socket, baked into the def as
    /// a constant from the node weight. For inputs plyphon requires to be
    /// compile-time constants (a delay's `maxdelay` sizes its delay line, a
    /// limiter's `dur` its look-ahead buffer) or latches at unit init (`Line`).
    /// Editing one re-derives the synthdef.
    Init {
        /// The value's name: its inspector label and sugar keyword.
        name: &'static str,
        /// The value a fresh node starts at.
        default: f32,
        /// The inspector row's doc line.
        doc: &'static str,
    },
}

impl In {
    /// The entry's socket/inspector name, if it has one (`Baked` does not).
    pub fn name(&self) -> Option<&'static str> {
        match self {
            In::Signal { name, .. } | In::Param { name, .. } | In::Init { name, .. } => Some(name),
            In::Baked(_) => None,
        }
    }

    /// Whether this entry is a socket (a dsp input port).
    pub fn is_socket(&self) -> bool {
        matches!(self, In::Signal { .. } | In::Param { .. })
    }
}

/// One wrapped plyphon unit generator: the descriptor a
/// [`UnitNode`](crate::UnitNode) (identified by [`unit`](Self::unit)) is
/// driven by.
#[derive(Clone, Copy, Debug)]
pub struct UnitDesc {
    /// The `.gantz` keyword and palette name, e.g. `"~lpf"`.
    pub keyword: &'static str,
    /// The plyphon unit name, e.g. `"LPF"` - the node's stored identity and
    /// the emitted [`UnitSpec`](plyphon::synthdef::UnitSpec) name.
    pub unit: &'static str,
    /// One entry per plyphon input, in plyphon input order.
    pub inputs: &'static [In],
    /// One doc line per unit output (the node's dsp output ports).
    pub outputs: &'static [&'static str],
    /// The palette/inspector description.
    pub doc: &'static str,
}

impl UnitDesc {
    /// The socketed entries (`Signal`/`Param`) in socket order.
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

/// The descriptor with the given `.gantz` keyword (e.g. `"~lpf"`), if any.
pub fn unit_desc_by_keyword(keyword: &str) -> Option<&'static UnitDesc> {
    UNITS.iter().find(|d| d.keyword == keyword)
}

/// A [`In::Signal`] row entry.
const fn sig(name: &'static str, doc: &'static str) -> In {
    In::Signal { name, doc }
}

/// A [`In::Param`] (hybrid) row entry.
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
        inputs,
        outputs,
        doc,
    }
}

/// A hybrid oscillator/filter frequency param.
const fn freq(default: f32, doc: &'static str) -> In {
    par("freq", default, 0.0, 20_000.0, " Hz", doc)
}

/// The wrapped plyphon units, grouped by family. Signatures follow the
/// published plyphon crate the workspace pins (SC-conventional arg order).
pub static UNITS: &[UnitDesc] = &[
    // --- Oscillators (band-limited + LF) ---
    u(
        "~saw",
        "Saw",
        &[freq(220.0, "frequency; a wire drives it directly")],
        &["sawtooth signal"],
        "Band-limited sawtooth oscillator",
    ),
    u(
        "~pulse",
        "Pulse",
        &[
            freq(220.0, "frequency; a wire drives it directly"),
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
        &[freq(220.0, "frequency; a wire drives it (FM)"), baked(0.0)],
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
    // --- Noise ---
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
    // --- Filters ---
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
    // --- Delays ---
    u(
        "~delayn",
        "DelayN",
        &[
            sig("in", "signal to delay"),
            init(
                "maxdelay",
                0.2,
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
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
                "max delay time (sizes the delay line; re-derives)",
            ),
            par("delay", 0.2, 0.0, 10.0, " s", "delay time"),
            par("decay", 1.0, -60.0, 60.0, " s", "60 dB feedback decay time"),
        ],
        &["all-passed signal"],
        "All-pass (phase-dispersing feedback) delay, cubic interpolation",
    ),
    // --- Lines ---
    u(
        "~line",
        "Line",
        &[
            init("start", 0.0, "start value (latched at spawn; re-derives)"),
            init("end", 1.0, "end value (latched at spawn; re-derives)"),
            init(
                "dur",
                1.0,
                "ramp duration in seconds (latched at spawn; re-derives)",
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
                "start value, nonzero (latched at spawn; re-derives)",
            ),
            init(
                "end",
                2.0,
                "end value, same sign (latched at spawn; re-derives)",
            ),
            init(
                "dur",
                1.0,
                "ramp duration in seconds (latched at spawn; re-derives)",
            ),
            baked(0.0),
        ],
        &["exponential ramp signal"],
        "Exponential ramp from start to end (values latch when the synth spawns)",
    ),
    // --- Dynamics ---
    u(
        "~limiter",
        "Limiter",
        &[
            sig("in", "signal to limit"),
            par("level", 1.0, 0.0, 2.0, "", "peak output amplitude"),
            init(
                "dur",
                0.01,
                "look-ahead time (sizes the buffer; re-derives)",
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
                "look-ahead time (sizes the buffer; re-derives)",
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
    // --- Pan / mix ---
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
    // --- Math / range ---
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
                names.contains(desc.unit),
                "unit `{}` is not registered in plyphon",
                desc.unit
            );
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
}
