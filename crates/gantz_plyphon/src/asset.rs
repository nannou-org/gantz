//! Audio sample data as a content-addressed asset.
//!
//! [`AudioAsset`] is the DSP domain's interpretation of the bytes in the
//! [`BUFFER_SECTION`] blob store. It is canonical decoded PCM, interleaved
//! `f32` samples plus a channel count and sample rate. Its
//! [`encode`](AudioAsset::encode) produces a deterministic byte layout, so
//! identical audio always yields the same [`gantz_ca::ContentAddr`]
//! regardless of the source file format. A stored blob is self-verifying,
//! since re-decoding then re-encoding reproduces the key. The bytes convert
//! straight into a [`plyphon::Buffer`] for installation into the engine's
//! buffer table. The engine itself never decodes anything.
//!
//! The registry core stores opaque, codec-agnostic bytes, addressed as the
//! raw blake3 of the encoding per the blob addressing rule. The audio domain
//! alone defines what those bytes mean, via [`AudioBuffers`] and the typed
//! accessors here.

use gantz_ca::{BlobDecl, BlobLiveness, ContentAddr, Registry};
use thiserror::Error;

/// The audio-buffer blob section, the DSP domain's content-addressed store
/// of canonically encoded PCM.
///
/// Blobs are kept alive by content references such as a `~playbuf` node's
/// [`Node::required_blobs`](gantz_core::Node::required_blobs), so export and
/// prune carry exactly the buffers live graphs use.
pub struct AudioBuffers;

/// The id of the audio-buffer blob section.
pub const BUFFER_SECTION: &str = "dsp.buffer";

/// The canonical-encoding tag, bumped if the byte layout ever changes.
const VERSION: u8 = 1;
/// Bytes of fixed header before the sample data. A 1-byte version, 4-byte
/// channel count, 8-byte sample rate and 8-byte sample count.
const HEADER_LEN: usize = 1 + 4 + 8 + 8;

/// Canonical decoded PCM, frame-major interleaved `f32` samples plus a
/// channel count and the data's own sample rate.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioAsset {
    /// `num_frames * num_channels` samples, interleaved frame-major.
    samples: Vec<f32>,
    /// Number of channels, at least 1.
    num_channels: usize,
    /// The data's own sample rate in Hz.
    sample_rate: f64,
}

/// A failure decoding blob bytes into an [`AudioAsset`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecodeError {
    /// The blob is shorter than the fixed header, or its declared sample count
    /// disagrees with the trailing bytes.
    #[error("malformed audio asset: {0}")]
    Malformed(&'static str),
    /// The blob's version tag is not one this build understands.
    #[error("unsupported audio asset version: {0}")]
    UnsupportedVersion(u8),
}

impl AudioAsset {
    /// Build an asset from interleaved samples. `num_channels` is clamped to at
    /// least 1. Any trailing partial frame is dropped so `samples.len()` is an
    /// exact multiple of the channel count.
    pub fn from_interleaved(samples: Vec<f32>, num_channels: usize, sample_rate: f64) -> Self {
        let num_channels = num_channels.max(1);
        let mut samples = samples;
        let len = samples.len() - (samples.len() % num_channels);
        samples.truncate(len);
        AudioAsset {
            samples,
            num_channels,
            sample_rate,
        }
    }

    /// The frame-major interleaved samples.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// The number of channels, at least 1.
    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    /// The number of frames, samples per channel.
    pub fn num_frames(&self) -> usize {
        self.samples.len() / self.num_channels
    }

    /// The data's own sample rate in Hz.
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Encode the asset into its canonical bytes, the bytes whose raw blake3
    /// hash is its content address.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.samples.len() * 4);
        bytes.push(VERSION);
        bytes.extend_from_slice(&(self.num_channels as u32).to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(self.samples.len() as u64).to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    /// The asset's content address, the raw blake3 hash of its canonical
    /// encoding.
    pub fn addr(&self) -> ContentAddr {
        gantz_ca::blob_addr(&self.encode())
    }

    /// Decode canonical bytes back into an asset.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < HEADER_LEN {
            return Err(DecodeError::Malformed("shorter than header"));
        }
        let version = bytes[0];
        if version != VERSION {
            return Err(DecodeError::UnsupportedVersion(version));
        }
        let num_channels = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
        let sample_rate = f64::from_le_bytes(bytes[5..13].try_into().unwrap());
        let num_samples = u64::from_le_bytes(bytes[13..21].try_into().unwrap()) as usize;
        let sample_bytes = &bytes[HEADER_LEN..];
        if sample_bytes.len() != num_samples * 4 {
            return Err(DecodeError::Malformed("sample count mismatch"));
        }
        if num_channels == 0 {
            return Err(DecodeError::Malformed("zero channels"));
        }
        let samples = sample_bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Ok(AudioAsset {
            samples,
            num_channels,
            sample_rate,
        })
    }
}

impl BlobDecl for AudioBuffers {
    const ID: &'static str = BUFFER_SECTION;
    const LIVENESS: BlobLiveness = BlobLiveness::ContentReferenced;
}

impl From<AudioAsset> for plyphon::Buffer {
    fn from(asset: AudioAsset) -> Self {
        plyphon::Buffer::from_interleaved(asset.samples, asset.num_channels, asset.sample_rate)
    }
}

/// Insert the asset's canonical encoding into the registry's audio-buffer
/// blob section, returning its content address. Idempotent, since identical
/// audio always lands at the same address.
pub fn add_audio_asset(reg: &mut Registry, asset: &AudioAsset) -> ContentAddr {
    reg.add_blob(AudioBuffers::ID, AudioBuffers::LIVENESS, asset.encode())
}

/// Decode the audio asset stored at the given address in the registry's
/// audio-buffer blob section, if present and well formed.
pub fn audio_asset(reg: &Registry, addr: &ContentAddr) -> Option<AudioAsset> {
    let bytes = reg.blob(AudioBuffers::ID, addr)?;
    AudioAsset::decode(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> AudioAsset {
        AudioAsset::from_interleaved(vec![0.0, 1.0, -1.0, 0.5, 0.25, -0.75], 2, 44_100.0)
    }

    #[test]
    fn encode_decode_round_trips() {
        let a = asset();
        let decoded = AudioAsset::decode(&a.encode()).unwrap();
        assert_eq!(decoded, a);
    }

    #[test]
    fn addr_is_stable_and_metadata_sensitive() {
        let a = asset();
        // Deterministic across re-encodes.
        assert_eq!(a.addr(), a.addr());
        // A different sample rate is a different asset, since metadata is in
        // the hash.
        let b = AudioAsset::from_interleaved(a.samples().to_vec(), a.num_channels(), 48_000.0);
        assert_ne!(a.addr(), b.addr());
        // A different channel interpretation of the same samples differs too.
        let c = AudioAsset::from_interleaved(a.samples().to_vec(), 1, a.sample_rate());
        assert_ne!(a.addr(), c.addr());
    }

    #[test]
    fn trailing_partial_frame_is_dropped() {
        let a = AudioAsset::from_interleaved(vec![1.0, 2.0, 3.0], 2, 48_000.0);
        assert_eq!(a.num_frames(), 1);
        assert_eq!(a.samples(), &[1.0, 2.0]);
    }

    #[test]
    fn converts_to_a_plyphon_buffer() {
        let a = asset();
        let buffer: plyphon::Buffer = a.clone().into();
        assert_eq!(buffer.num_channels(), a.num_channels());
        assert_eq!(buffer.num_frames(), a.num_frames());
        assert_eq!(buffer.sample_rate(), a.sample_rate());
    }

    #[test]
    fn decode_rejects_truncated_and_mismatched_blobs() {
        assert_eq!(
            AudioAsset::decode(&[1, 2, 3]),
            Err(DecodeError::Malformed("shorter than header")),
        );
        // Header claims two samples but no sample bytes follow.
        let mut bytes = vec![VERSION];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&48_000f64.to_le_bytes());
        bytes.extend_from_slice(&2u64.to_le_bytes());
        assert_eq!(
            AudioAsset::decode(&bytes),
            Err(DecodeError::Malformed("sample count mismatch")),
        );
    }

    #[test]
    fn registry_round_trip_through_the_buffer_section() {
        let mut reg = Registry::default();
        let a = asset();
        let addr = add_audio_asset(&mut reg, &a);
        assert_eq!(addr, a.addr());
        // The address is the raw blake3 of the canonical bytes, the
        // iroh-compatible blob addressing rule.
        assert_eq!(addr, gantz_ca::blob_addr(&a.encode()));
        assert_eq!(audio_asset(&reg, &addr), Some(a));
        // Idempotent re-add.
        let again = add_audio_asset(&mut reg, &asset());
        assert_eq!(again, addr);
    }
}
