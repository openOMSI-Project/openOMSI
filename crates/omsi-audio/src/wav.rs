//! Minimal RIFF/WAVE PCM reader (8/16/24/32-bit integer and 32-bit float).

use anyhow::{anyhow, Result};

pub struct WavData {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples as 16-bit integers (-32768..32767 for -1..1): half the memory of
    /// floats, and what nearly every OMSI sound is anyway (the decoded sounds of a big map
    /// with its fleet took 320 MB as floats).
    pub samples: Vec<i16>,
}

fn quantize(x: f32) -> i16 {
    (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

pub fn parse_wav(bytes: &[u8]) -> Result<WavData> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(anyhow!("not a RIFF/WAVE file"));
    }
    let mut p = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // format tag, channels, rate, bits
    let mut data: Option<&[u8]> = None;
    while p + 8 <= bytes.len() {
        let id = &bytes[p..p + 4];
        let size = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
        let body_end = (p + 8 + size).min(bytes.len());
        let body = &bytes[p + 8..body_end];
        match id {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let ch = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([body[14], body[15]]);
                fmt = Some((tag, ch, rate, bits));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        p = p + 8 + size + (size & 1);
    }
    let (tag, channels, sample_rate, bits) = fmt.ok_or_else(|| anyhow!("missing fmt chunk"))?;
    let data = data.ok_or_else(|| anyhow!("missing data chunk"))?;
    let samples: Vec<i16> = match (tag, bits) {
        (1, 8) | (0xFFFE, 8) => data.iter().map(|b| ((*b as i16) - 128) << 8).collect(),
        (1, 16) | (0xFFFE, 16) => data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect(),
        (1, 24) | (0xFFFE, 24) => data.chunks_exact(3).map(|c| i16::from_le_bytes([c[1], c[2]])).collect(),
        (1, 32) | (0xFFFE, 32) => data.chunks_exact(4).map(|c| i16::from_le_bytes([c[2], c[3]])).collect(),
        (3, 32) => data.chunks_exact(4).map(|c| quantize(f32::from_le_bytes(c.try_into().unwrap()))).collect(),
        _ => return Err(anyhow!("unsupported WAV format tag {tag} / {bits} bit")),
    };
    Ok(WavData { sample_rate, channels: channels.max(1), samples })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(tag: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&44100u32.to_le_bytes());
        b.extend_from_slice(&(44100u32 * bits as u32 / 8).to_le_bytes());
        b.extend_from_slice(&(bits / 8).to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn every_format_becomes_16_bit() {
        // 8-bit is unsigned around 128
        assert_eq!(parse_wav(&wav(1, 8, &[0, 128, 255])).unwrap().samples, vec![-32768, 0, 32512]);
        assert_eq!(parse_wav(&wav(1, 16, &[0x00, 0x80, 0xff, 0x7f])).unwrap().samples, vec![-32768, 32767]);
        // 24 and 32 bit keep their top 16 bits
        assert_eq!(parse_wav(&wav(1, 24, &[0x11, 0x00, 0x40])).unwrap().samples, vec![0x4000]);
        assert_eq!(parse_wav(&wav(1, 32, &[0x11, 0x22, 0x00, 0xc0])).unwrap().samples, vec![-0x4000]);
        let f: Vec<u8> = [0.5f32, -1.0, 2.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        assert_eq!(parse_wav(&wav(3, 32, &f)).unwrap().samples, vec![16384, -32768, 32767]);
    }
}

/// An Ogg Vorbis or FLAC file decoded whole (the ambience's recordings ship as FLAC: half
/// the size of the WAV, nothing lost).
pub fn parse_compressed(bytes: &[u8]) -> Result<WavData> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(if bytes.starts_with(b"fLaC") { "flac" } else { "ogg" });
    let mut format = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?.format;
    let track = format.default_track().ok_or_else(|| anyhow!("no audio track"))?.clone();
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(0);
    let mut samples = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        channels = spec.channels.count() as u16;
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * spec.channels.count() {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        samples.extend(b.samples().iter().map(|x| quantize(*x)));
    }
    if rate == 0 || channels == 0 {
        return Err(anyhow!("no sound in the file"));
    }
    Ok(WavData { sample_rate: rate, channels, samples })
}
