use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use typed_floats::{NonNaNFinite, PositiveFinite};
use voicevox_core::{
    AccentPhrase, AudioQuery, FRAME_RATE, StyleId,
    blocking::{Onnxruntime, OpenJtalk, Synthesizer, VoiceModelFile},
};

use crate::assets::{Assets, ResolvedVoice};
use crate::wav::PcmFormat;

/// AudioQuery parameters. `None` keeps VOICEVOX's default value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intonation: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_phoneme_length: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_phoneme_length: Option<f32>,
}

impl VoiceParams {
    /// Fills unset values in `self` with values from `base`.
    pub fn or(self, base: Self) -> Self {
        Self {
            speed: self.speed.or(base.speed),
            pitch: self.pitch.or(base.pitch),
            intonation: self.intonation.or(base.intonation),
            volume: self.volume.or(base.volume),
            pre_phoneme_length: self.pre_phoneme_length.or(base.pre_phoneme_length),
            post_phoneme_length: self.post_phoneme_length.or(base.post_phoneme_length),
        }
    }

    /// Human-readable summary of the values that are set, e.g. `speed=1.2, pitch=0.05`.
    pub fn describe(&self) -> String {
        [
            ("speed", self.speed),
            ("pitch", self.pitch),
            ("intonation", self.intonation),
            ("volume", self.volume),
            ("pre_phoneme_length", self.pre_phoneme_length),
            ("post_phoneme_length", self.post_phoneme_length),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|v| format!("{name}={v}")))
        .collect::<Vec<_>>()
        .join(", ")
    }

    /// Rejects values that VOICEVOX's types accept but that cannot produce sensible audio.
    /// In particular `speed` must be strictly positive: phoneme lengths are divided by it, so `0`
    /// makes the audio infinitely long and synthesis tries to allocate unbounded memory.
    pub fn validate(&self) -> Result<()> {
        fn check(name: &str, value: Option<f32>, min: f32, allow_min: bool) -> Result<()> {
            if let Some(v) = value
                && !(v.is_finite() && (v > min || (allow_min && v == min)))
            {
                let expected = if allow_min { "or greater" } else { "exclusive" };
                bail!("--{name} must be a finite number above {min} ({expected}), got {v}");
            }
            Ok(())
        }

        check("speed", self.speed, 0.0, false)?;
        check("pitch", self.pitch, f32::MIN, true)?;
        check("intonation", self.intonation, f32::MIN, true)?;
        check("volume", self.volume, 0.0, true)?;
        check("pre-phoneme-length", self.pre_phoneme_length, 0.0, true)?;
        check("post-phoneme-length", self.post_phoneme_length, 0.0, true)?;

        // Silence is stretched by speed like speech, and is held in memory as zero samples.
        let speed = self.speed.unwrap_or(1.0);
        for (name, value) in [
            ("pre-phoneme-length", self.pre_phoneme_length),
            ("post-phoneme-length", self.post_phoneme_length),
        ] {
            if let Some(v) = value
                && v / speed > MAX_SILENCE_SECONDS
            {
                bail!(
                    "--{name} would make {:.0} s of silence (limit {MAX_SILENCE_SECONDS:.0} s at --speed {speed})",
                    v / speed
                );
            }
        }
        Ok(())
    }

    fn apply(&self, query: &mut AudioQuery) -> Result<()> {
        self.validate()?;

        fn positive(name: &str, value: f32) -> Result<PositiveFinite<f32>> {
            PositiveFinite::<f32>::new(value)
                .map_err(|_| anyhow::anyhow!("--{name} must be a positive finite number: {value}"))
        }
        fn finite(name: &str, value: f32) -> Result<NonNaNFinite<f32>> {
            NonNaNFinite::<f32>::new(value)
                .map_err(|_| anyhow::anyhow!("--{name} must be a finite number: {value}"))
        }

        if let Some(v) = self.speed {
            query.speed_scale = positive("speed", v)?;
        }
        if let Some(v) = self.pitch {
            query.pitch_scale = finite("pitch", v)?;
        }
        if let Some(v) = self.intonation {
            query.intonation_scale = finite("intonation", v)?;
        }
        if let Some(v) = self.volume {
            query.volume_scale = positive("volume", v)?;
        }
        if let Some(v) = self.pre_phoneme_length {
            query.pre_phoneme_length = positive("pre-phoneme-length", v)?;
        }
        if let Some(v) = self.post_phoneme_length {
            query.post_phoneme_length = positive("post-phoneme-length", v)?;
        }
        Ok(())
    }
}

pub struct Engine {
    synthesizer: Synthesizer<OpenJtalk>,
    style_id: StyleId,
    params: VoiceParams,
}

impl Engine {
    /// Initializes ONNX Runtime and loads only the VVM that contains the requested voice.
    pub fn new(assets: &Assets, voice: &ResolvedVoice, params: VoiceParams) -> Result<Self> {
        let onnxruntime = Onnxruntime::load_once()
            .filename(&assets.onnxruntime)
            .perform()
            .with_context(|| {
                format!(
                    "Failed to load ONNX Runtime from {}",
                    assets.onnxruntime.display()
                )
            })?;
        let dict = assets
            .dict
            .to_str()
            .context("Dictionary path is not valid UTF-8")?;
        let open_jtalk = OpenJtalk::new(dict).context("Failed to load Open JTalk dictionary")?;
        let synthesizer = Synthesizer::builder(onnxruntime)
            .text_analyzer(open_jtalk)
            .build()?;

        let model = VoiceModelFile::open(&voice.vvm)
            .with_context(|| format!("Failed to open voice model {}", voice.vvm.display()))?;
        synthesizer.load_voice_model(&model).perform()?;

        Ok(Self {
            synthesizer,
            style_id: voice.style_id,
            params,
        })
    }

    /// Synthesizes `text` and passes its PCM samples to `sink` piece by piece as they are ready.
    ///
    /// The audio length is known from the AudioQuery before the expensive synthesis, so long
    /// sentences are synthesized in segments split at accent phrase boundaries (see
    /// [`plan_segments`]) to bound memory. Intonation is still predicted from the whole sentence.
    /// Each segment is rendered in growing slices ([`SLICE_SECONDS`]) so that the first audio is
    /// available quickly; VOICEVOX CORE renders slices with margins, and the joined result is
    /// identical to rendering the segment at once.
    /// Leading/trailing silence beyond [`EDGE_SILENCE_SECONDS`] is emitted as zero samples
    /// instead of being synthesized.
    pub fn synthesize(
        &self,
        text: &str,
        sink: &mut dyn FnMut(PcmFormat, &[u8]) -> Result<()>,
    ) -> Result<()> {
        let mut query = self
            .synthesizer
            .create_audio_query(text, self.style_id)
            .with_context(|| format!("Failed to analyze text: {text}"))?;
        self.params.apply(&mut query)?;

        let format = PcmFormat {
            channels: if query.output_stereo { 2 } else { 1 },
            sample_rate: query.output_sampling_rate.get().get(),
            bits_per_sample: 16,
        };
        let speed = query.speed_scale.get();
        let pre = query.pre_phoneme_length.get();
        let post = query.post_phoneme_length.get();
        let synthesized_pre = pre.min(EDGE_SILENCE_SECONDS);
        let synthesized_post = post.min(EDGE_SILENCE_SECONDS);

        let mut template = query.clone();
        template.accent_phrases.clear();
        template.kana = None;
        let frames = |phrases: &[AccentPhrase], pre: f32, post: f32| -> Result<usize> {
            Ok(sub_query(&template, phrases, pre, post)?
                .frame_length()
                .calculate()
                .0)
        };
        let phrases = &query.accent_phrases;
        let phrase_frames = phrases
            .iter()
            .map(|phrase| frames(std::slice::from_ref(phrase), 0.0, 0.0))
            .collect::<Result<Vec<_>>>()?;
        let pauses: Vec<bool> = phrases.iter().map(|p| p.pause_mora.is_some()).collect();
        let edges = (
            frames(&[], synthesized_pre, 0.0)?,
            frames(&[], 0.0, synthesized_post)?,
        );
        let segments = plan_segments(&phrase_frames, &pauses, edges, max_segment_frames())?;

        let leading = silence(format, (pre - synthesized_pre) / speed);
        if !leading.is_empty() {
            sink(format, &leading)?;
        }
        for (i, range) in segments.iter().enumerate() {
            let segment = sub_query(
                &template,
                &phrases[range.clone()],
                if i == 0 { synthesized_pre } else { 0.0 },
                if i + 1 == segments.len() {
                    synthesized_post
                } else {
                    0.0
                },
            )?;
            let feature = self
                .synthesizer
                .create_audio_feature(&segment, self.style_id)
                .perform()
                .with_context(|| format!("Failed to synthesize: {text}"))?;
            let mut offset = 0;
            for slice in 0.. {
                if offset >= feature.len() {
                    break;
                }
                let end = (offset + slice_frames(slice)).min(feature.len());
                let pcm = self
                    .synthesizer
                    .render(feature.slice(offset..end))
                    .with_context(|| format!("Failed to synthesize: {text}"))?;
                sink(format, &pcm)?;
                offset = end;
            }
        }
        let trailing = silence(format, (post - synthesized_post) / speed);
        if !trailing.is_empty() {
            sink(format, &trailing)?;
        }
        Ok(())
    }
}

/// Lengths of the slices rendered at a time, growing so that the first audio comes quickly
/// (measured on a 21.6 s sentence: first audio after 132 ms with 0.5 s slices vs. 2.2 s for the
/// whole) while later slices stay large to keep the per-call overhead low.
const SLICE_SECONDS: [f64; 3] = [0.5, 1.0, 3.0];

fn slice_frames(index: usize) -> usize {
    let seconds = SLICE_SECONDS[index.min(SLICE_SECONDS.len() - 1)];
    (seconds * FRAME_RATE).round() as usize
}

/// Upper bound of the audio length synthesized in one call.
///
/// Memory use of VOICEVOX CORE grows faster than linearly with the audio length of one call
/// (measured peak RSS: 12 s ≈ 0.7 GB, 47 s ≈ 2.5 GB, 95 s ≈ 6.6 GB; most of it in
/// `create_audio_feature`, so rendering in slices alone does not help). Longer sentences are therefore
/// split into segments of at most this length.
const MAX_SEGMENT_SECONDS: f64 = 30.0;

/// Leading/trailing silence that is synthesized (VOICEVOX's default pre/post phoneme length).
/// Anything longer is appended as zero samples.
const EDGE_SILENCE_SECONDS: f32 = 0.1;

/// Upper bound of the leading/trailing silence, which is held in memory as zero samples.
const MAX_SILENCE_SECONDS: f32 = 60.0;

fn max_segment_frames() -> usize {
    (MAX_SEGMENT_SECONDS * FRAME_RATE) as usize
}

/// A copy of `template` (an AudioQuery without accent phrases) with the given phrases and silence.
fn sub_query(
    template: &AudioQuery,
    phrases: &[AccentPhrase],
    pre: f32,
    post: f32,
) -> Result<AudioQuery> {
    let length = |secs: f32| {
        PositiveFinite::<f32>::new(secs)
            .map_err(|_| anyhow::anyhow!("Invalid phoneme length: {secs}"))
    };
    let mut query = template.clone();
    query.accent_phrases = phrases.to_vec();
    query.pre_phoneme_length = length(pre)?;
    query.post_phoneme_length = length(post)?;
    Ok(query)
}

fn silence(format: PcmFormat, seconds: f32) -> Vec<u8> {
    let samples = (f64::from(seconds) * f64::from(format.sample_rate)).round() as usize;
    vec![0; samples * usize::from(format.channels) * usize::from(format.bits_per_sample / 8)]
}

/// Groups accent phrases into segments of at most `limit` frames.
///
/// `phrase_frames[i]` is the frame length of phrase `i`, `pauses[i]` whether a pause follows it,
/// and `edges` the frames of the silence synthesized before the first and after the last phrase.
/// Each segment takes as many phrases as fit, then backs off to the last phrase followed by a
/// pause (if any) so that segments are joined at silence rather than in the middle of speech.
pub fn plan_segments(
    phrase_frames: &[usize],
    pauses: &[bool],
    edges: (usize, usize),
    limit: usize,
) -> Result<Vec<Range<usize>>> {
    let n = phrase_frames.len();
    let trailing = |end: usize| if end == n { edges.1 } else { 0 };
    let mut segments = Vec::new();
    let mut start = 0;

    while start < n {
        let mut total = if start == 0 { edges.0 } else { 0 } + phrase_frames[start];
        if total + trailing(start + 1) > limit {
            bail!(
                "One accent phrase would be too long ({:.0} s, limit {MAX_SEGMENT_SECONDS:.0} s). Check --speed.",
                (total + trailing(start + 1)) as f64 / FRAME_RATE
            );
        }
        let mut end = start + 1;
        while end < n && total + phrase_frames[end] + trailing(end + 1) <= limit {
            total += phrase_frames[end];
            end += 1;
        }
        if end < n
            && let Some(at_pause) = (start + 1..=end).rev().find(|&b| pauses[b - 1])
        {
            end = at_pause;
        }
        segments.push(start..end);
        start = end;
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_speed_is_rejected() {
        let params = VoiceParams {
            speed: Some(0.0),
            ..Default::default()
        };
        assert!(params.validate().is_err());
        // The reason: VOICEVOX's type for speed_scale accepts 0.0, which makes the audio endless.
        let mut query = AudioQuery::from(vec![]);
        query.speed_scale = PositiveFinite::<f32>::new(0.0).unwrap();
        assert_eq!(query.frame_length().calculate().0, usize::MAX);
    }

    #[test]
    fn validate_accepts_boundaries_and_rejects_non_finite() {
        let ok = VoiceParams {
            speed: Some(0.5),
            pitch: Some(-0.15),
            intonation: Some(0.0),
            volume: Some(0.0),
            pre_phoneme_length: Some(0.0),
            post_phoneme_length: Some(0.0),
        };
        assert!(ok.validate().is_ok());
        for bad in [
            VoiceParams {
                pitch: Some(f32::NAN),
                ..Default::default()
            },
            VoiceParams {
                volume: Some(-1.0),
                ..Default::default()
            },
            VoiceParams {
                speed: Some(f32::INFINITY),
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn overly_long_silence_is_rejected() {
        let params = |pre, speed| VoiceParams {
            pre_phoneme_length: Some(pre),
            speed,
            ..Default::default()
        };
        assert!(params(1000.0, None).validate().is_err());
        assert!(params(30.0, None).validate().is_ok());
        // Silence is stretched by speed too.
        assert!(params(30.0, Some(0.25)).validate().is_err());
    }

    fn ranges(segments: Vec<Range<usize>>) -> Vec<(usize, usize)> {
        segments.into_iter().map(|r| (r.start, r.end)).collect()
    }

    #[test]
    fn short_sentence_is_one_segment() {
        let segments = plan_segments(&[10, 10, 10], &[false; 3], (5, 5), 100).unwrap();
        assert_eq!(ranges(segments), [(0, 3)]);
    }

    #[test]
    fn empty_sentence_has_no_segments() {
        assert!(plan_segments(&[], &[], (5, 5), 100).unwrap().is_empty());
    }

    #[test]
    fn long_sentence_is_split_at_pauses() {
        // Pauses after phrase 1 and 4. Phrases 0..=2 (30 frames) would fit, but the split moves
        // back to the pause after phrase 1.
        let pauses = [false, true, false, false, true, false];
        let segments = plan_segments(&[10; 6], &pauses, (0, 0), 35).unwrap();
        assert_eq!(ranges(segments), [(0, 2), (2, 5), (5, 6)]);
    }

    #[test]
    fn long_sentence_without_pauses_is_split_where_full() {
        let segments = plan_segments(&[10; 5], &[false; 5], (0, 0), 25).unwrap();
        assert_eq!(ranges(segments), [(0, 2), (2, 4), (4, 5)]);
    }

    #[test]
    fn edge_silence_counts_toward_first_and_last_segments() {
        // 3 phrases of 10 frames fit in 30 only without the 5-frame edges.
        let segments = plan_segments(&[10; 3], &[false; 3], (5, 5), 30).unwrap();
        assert_eq!(ranges(segments), [(0, 2), (2, 3)]);
    }

    #[test]
    fn phrase_longer_than_limit_is_an_error() {
        assert!(plan_segments(&[10, 50, 10], &[false; 3], (0, 0), 30).is_err());
    }

    #[test]
    fn or_prefers_self() {
        let cli = VoiceParams {
            speed: Some(1.5),
            ..Default::default()
        };
        let preset = VoiceParams {
            speed: Some(1.1),
            pitch: Some(0.05),
            ..Default::default()
        };
        let merged = cli.or(preset);
        assert_eq!(merged.speed, Some(1.5));
        assert_eq!(merged.pitch, Some(0.05));
        assert_eq!(merged.volume, None);
    }

    #[test]
    fn describe_lists_only_set_values() {
        let params = VoiceParams {
            speed: Some(1.2),
            volume: Some(0.8),
            ..Default::default()
        };
        assert_eq!(params.describe(), "speed=1.2, volume=0.8");
    }
}
