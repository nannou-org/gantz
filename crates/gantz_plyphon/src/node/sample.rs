//! The `~sample` buffer source node.

use std::hash::{Hash, Hasher};

use gantz_ca::{ContentAddr, Datum};
use gantz_nodedef::{ExprArgs, Meta, View};
use gantz_nodetag::NodeTag;
use gantz_ui::{self as ui, Edit, Element};
use plyphon::synthdef::InputRef;
use serde::{Deserialize, Serialize};

use crate::dsp::{BufferSource, DspBuilder, NodeDsp, Signal, ToNodeDsp};
use crate::view::{doc, row};

/// An audio asset as a buffer source. It emits the bufnum wire of the asset
/// for buffer sockets, such as the one on `~playbuf`.
///
/// The node holds the address of the asset and caches its channel count,
/// frame count and sample rate. The samples live in the asset store. The
/// audio driver installs the asset once and shares it read-only. Writers
/// cannot use it.
///
/// An unassigned node emits the bufnum `-1`, which reads an empty buffer.
/// Steel-inert like the other dsp nodes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, NodeTag)]
pub struct Sample {
    /// The audio asset, or `None` until one is assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset: Option<ContentAddr>,
    /// The cached channel count. It sets the output count of a reader.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    channels: usize,
    /// The cached frame count, for display.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    frames: usize,
    /// The cached sample rate in Hz, for display.
    #[serde(default, skip_serializing_if = "crate::node::is_default")]
    sample_rate: f64,
}

/// The name of the DSP domain's importer of WAV files.
pub const WAV_IMPORTER: &str = "plyphon.wav";

/// The edit that loads an imported file into a `~sample`.
const LOAD: &str = "load";

/// The keys of the data that [`import_wav`] gives.
const CHANNELS: &str = "channels";
const FRAMES: &str = "frames";
const SAMPLE_RATE: &str = "sample_rate";

impl Sample {
    /// A node for `asset`, whose PCM has `channels` channels of `frames`
    /// frames at `sample_rate` Hz.
    pub fn new(asset: ContentAddr, channels: usize, frames: usize, sample_rate: f64) -> Self {
        Sample {
            asset: Some(asset),
            channels,
            frames,
            sample_rate,
        }
    }

    /// A node for the given asset, with its metadata cached.
    pub fn from_asset(asset: &crate::AudioAsset) -> Self {
        Sample::new(
            asset.addr(),
            asset.num_channels(),
            asset.num_frames(),
            asset.sample_rate(),
        )
    }

    /// The assigned asset, if any.
    pub fn asset(&self) -> Option<ContentAddr> {
        self.asset
    }

    /// The cached channel count, at least 1.
    pub fn channels(&self) -> usize {
        self.channels.max(1)
    }

    /// The cached frame count.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The cached sample rate in Hz.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

impl PartialEq for Sample {
    fn eq(&self, other: &Self) -> bool {
        self.asset == other.asset
            && self.channels == other.channels
            && self.frames == other.frames
            && self.sample_rate.to_bits() == other.sample_rate.to_bits()
    }
}

impl Eq for Sample {}

impl Hash for Sample {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.asset.hash(state);
        self.channels.hash(state);
        self.frames.hash(state);
        self.sample_rate.to_bits().hash(state);
    }
}

impl gantz_nodedef::NodeDef for Sample {
    const TAG: &'static str = <Self as NodeTag>::TAG;

    fn meta(&self) -> Meta {
        let blobs = self
            .asset
            .into_iter()
            .map(|addr| (crate::BUFFER_SECTION.to_string(), addr));
        Meta {
            name: "~sample".into(),
            description: "An audio sample as a read-only buffer, for example for `~playbuf`".into(),
            outputs: 1,
            blobs: blobs.collect(),
            ..Meta::default()
        }
    }

    fn expr(&self, _ctx: ExprArgs<'_>) -> Result<String, String> {
        // Steel-inert. A non-numeric placeholder output feeds the inert dsp
        // output edge, see the `NodeDsp` docs.
        Ok("'()".into())
    }

    fn view(&self) -> View {
        let summary = match self.asset {
            Some(asset) => {
                let hex = asset.to_string();
                let (channels, frames, rate) = (self.channels(), self.frames, self.sample_rate);
                format!(
                    "{}… {channels} ch, {frames} frames, {rate} Hz",
                    &hex[..hex.len().min(8)]
                )
            }
            None => "none".to_string(),
        };
        let load = ui::File {
            on: Some(Edit(LOAD.into())),
            import: Some(WAV_IMPORTER.into()),
            label: Some("load…".into()),
            tip: Some("load a WAV file into this sample".into()),
            key: None,
        };
        let summary = ui::Label {
            text: summary,
            ..ui::Label::default()
        };
        let asset = ui::Row {
            children: vec![Element::File(load), Element::Label(summary)],
            ..ui::Row::default()
        };
        let menu = gantz_nodedef::MenuItem {
            label: "load…".into(),
            tip: "load a WAV file into this sample".into(),
            edit: LOAD.into(),
            import: Some(WAV_IMPORTER.into()),
            checked: None,
        };
        View {
            outputs: vec![doc("buffer", "the sample, for a buffer socket")],
            rows: vec![row("asset", Element::Row(asset))],
            menu: vec![menu],
            ..View::default()
        }
    }

    fn edit(&self, edit: &str, value: &Datum) -> Option<Self> {
        let (asset, data) = gantz_nodedef::import::imported(value).filter(|_| edit == LOAD)?;
        let field = |key: &str| {
            let Datum::Map(entries) = data else {
                return None;
            };
            entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        };
        let count = |key: &str| field(key)?.as_f64().map(|n| n.max(0.0) as usize);
        let sample = Sample::new(
            asset,
            count(CHANNELS)?,
            count(FRAMES)?,
            field(SAMPLE_RATE)?.as_f64()?,
        );
        Some(sample)
    }
}

/// Import a WAV file as an audio asset in its canonical encoding, with the
/// data that a `~sample` keeps of it. The fn of the [`WAV_IMPORTER`].
pub fn import_wav(bytes: &[u8]) -> Result<(Vec<u8>, Datum), String> {
    let asset = crate::AudioAsset::from_wav(bytes).map_err(|e| e.to_string())?;
    let data = Datum::Map(vec![
        (
            CHANNELS.into(),
            Datum::from_u64(asset.num_channels() as u64),
        ),
        (FRAMES.into(), Datum::from_u64(asset.num_frames() as u64)),
        (SAMPLE_RATE.into(), Datum::F64(asset.sample_rate())),
    ]);
    Ok((asset.encode(), data))
}

impl NodeDsp for Sample {
    fn n_dsp_inputs(&self) -> usize {
        0
    }

    fn n_dsp_outputs(&self) -> usize {
        1
    }

    fn is_buffer_source(&self) -> bool {
        true
    }

    fn ugens(&self, path: &[usize], _inputs: &[Option<Signal>], b: &mut DspBuilder) -> Vec<Signal> {
        let bufnum = match self.asset {
            Some(asset) => b.push_buffer(path, BufferSource::Asset(asset), self.channels()),
            None => Signal::mono(InputRef::Constant(-1.0)),
        };
        vec![bufnum]
    }
}

impl ToNodeDsp for Sample {
    fn to_node_dsp(&self) -> Option<&dyn NodeDsp> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_sample_pushes_an_asset_binding() {
        let addr = gantz_ca::blob_addr(b"pcm");
        let mut b = DspBuilder::new(1);
        let outs = Sample::new(addr, 2, 64, 44_100.0).ugens(&[0], &[], &mut b);
        assert!(matches!(outs[0].channel(0), Some(InputRef::Param(_))));
        let finished = b.finish("t");
        assert_eq!(finished.buffers.len(), 1);
        assert_eq!(finished.buffers[0].source, BufferSource::Asset(addr));
        assert_eq!(finished.buffers[0].channels, 2);
        assert!(finished.def.units.is_empty(), "a source emits no units");
    }

    #[test]
    fn unassigned_sample_emits_minus_one() {
        let mut b = DspBuilder::new(1);
        let outs = Sample::default().ugens(&[0], &[], &mut b);
        assert!(matches!(outs[0].channel(0), Some(InputRef::Constant(v)) if v == -1.0));
        assert!(b.finish("t").buffers.is_empty());
    }

    #[test]
    fn required_blobs_surfaces_the_asset() {
        let addr = gantz_ca::blob_addr(b"x");
        let blobs = gantz_nodedef::NodeDef::meta(&Sample::new(addr, 1, 1, 1.0)).blobs;
        let blobs: Vec<_> = blobs
            .iter()
            .map(|(section, addr)| (section.clone(), addr.to_string()))
            .collect();
        assert_eq!(
            blobs,
            [(crate::BUFFER_SECTION.to_string(), addr.to_string())]
        );
        assert!(
            gantz_nodedef::NodeDef::meta(&Sample::default())
                .blobs
                .is_empty()
        );
    }
}
