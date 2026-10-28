use anyhow::{Result, bail};

/// Format of the PCM data inside a WAV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
}

/// Splits a PCM WAV file image into its format and raw sample data by scanning the RIFF chunks.
pub fn parse(wav: &[u8]) -> Result<(PcmFormat, &[u8])> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("Not a WAV file");
    }

    let mut format = None;
    let mut offset = 12;
    while offset + 8 <= wav.len() {
        let id = &wav[offset..offset + 4];
        let size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into()?) as usize;
        let body_start = offset + 8;
        let body_end = body_start.saturating_add(size).min(wav.len());
        let body = &wav[body_start..body_end];

        match id {
            b"fmt " if body.len() >= 16 => {
                let audio_format = u16::from_le_bytes([body[0], body[1]]);
                if audio_format != 1 {
                    bail!("Unsupported WAV encoding (format tag {audio_format})");
                }
                format = Some(PcmFormat {
                    channels: u16::from_le_bytes([body[2], body[3]]),
                    sample_rate: u32::from_le_bytes(body[4..8].try_into()?),
                    bits_per_sample: u16::from_le_bytes([body[14], body[15]]),
                });
            }
            b"data" => {
                let Some(format) = format else {
                    bail!("WAV data chunk appears before fmt chunk");
                };
                return Ok((format, body));
            }
            _ => {}
        }

        // Chunks are padded to an even size.
        offset = body_start + size + (size & 1);
    }

    bail!("WAV file has no data chunk")
}

/// Builds a WAV file image from 16-bit PCM samples.
pub fn encode(format: PcmFormat, pcm: &[u8]) -> Result<Vec<u8>> {
    if format.bits_per_sample != 16 || !(1..=2).contains(&format.channels) {
        bail!("Only 16-bit mono/stereo PCM is supported: {format:?}");
    }
    Ok(voicevox_core::wav_from_s16le(
        pcm,
        format.sample_rate,
        format.channels == 2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO_24K: PcmFormat = PcmFormat {
        channels: 1,
        sample_rate: 24000,
        bits_per_sample: 16,
    };

    #[test]
    fn roundtrip() {
        let pcm: Vec<u8> = (0..200u8).collect();
        let wav = encode(MONO_24K, &pcm).unwrap();
        let (format, data) = parse(&wav).unwrap();
        assert_eq!(format, MONO_24K);
        assert_eq!(data, pcm.as_slice());
    }

    #[test]
    fn skips_unknown_chunks() {
        let pcm = [1u8, 2, 3, 4];
        let wav = encode(MONO_24K, &pcm).unwrap();
        // Insert an odd-sized LIST chunk (with padding byte) between fmt and data.
        let data_pos = wav.windows(4).position(|w| w == b"data").unwrap();
        let mut patched = wav[..data_pos].to_vec();
        patched.extend_from_slice(b"LIST\x03\x00\x00\x00abc\x00");
        patched.extend_from_slice(&wav[data_pos..]);
        let (_, data) = parse(&patched).unwrap();
        assert_eq!(data, pcm);
    }

    #[test]
    fn rejects_non_wav() {
        assert!(parse(b"not a wav file").is_err());
    }
}
