use anyhow::{anyhow, Context};
use replaygain::ReplayGain;
use symphonia::core::audio::{SampleBuffer, SignalSpec};
use symphonia::core::codecs;
use symphonia::core::codecs::{CodecType, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, Track};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Track not found")]
    TrackNotFound,
    #[error("Metadata not found")]
    MetadataNotFound(String),
    #[error("Unsupported format")]
    FormatUnsupported,
    #[error("Cannot calculate duration")]
    ParsingDurationError,
    #[error("Cannot calculate gain/peak")]
    CalculatingGainPeakError,
    #[error("Error while call symphonia api")]
    Parse(SymphoniaError),
}

#[derive(Debug, Clone)]
pub struct PickedMetadata {
    pub format: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub bitrate: i32,
    pub bit_depth: i32,
    pub sample_rate: u32,
    pub duration_secs: u64,
    pub peak: f32,
    pub gain_db: f32
}

pub fn parse_and_validate(
    input: Box<dyn MediaSource>,
    file_name: Option<&str>,
) -> Result<PickedMetadata, ParseError> {
    let mut result = PickedMetadata {
        format: "".to_string(),
        title: None,
        artist: None,
        bitrate: 0,
        bit_depth: 0,
        sample_rate: 0,
        duration_secs: 0,
        peak: 0f32,
        gain_db: 0f32,
    };

    let media = MediaSourceStream::new(input, MediaSourceStreamOptions::default());
    let mut hint = Hint::default();

    /*if let Some((_, ext)) = file_name.and_then(|name| name.rsplit_once(".")) {
        debug!("File extension hint: {ext}");
        hint.with_extension(ext);
    } else {
        debug!("No hint");
    }*/

    let mut probed = symphonia::default::get_probe()
        .format(&hint, media, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|x| ParseError::Parse(x))?;

    let metadata = probed.format
        .metadata().current()
        .map(|x| x.to_owned())
        .or_else(|| {
            probed
                .metadata
                .get()
                .and_then(|m| m.current().map(|r| r.to_owned()))
        })
        .ok_or_else(|| ParseError::MetadataNotFound("root".to_string()))?;

    // Retrieve metadata
    for tag in metadata.tags() {
        if let Some(key) = tag.std_key {
            match key {
                StandardTagKey::TrackTitle => result.title = Some(tag.value.to_string()),
                StandardTagKey::Artist => result.artist = Some(tag.value.to_string()),
                // StandardTagKey::Arranger => {}
                // StandardTagKey::Bpm => {}
                _ => {
                    // Fuck, I can't get more useful information
                }
            }
        }
    }

    // TODO: Retrieve cover image
    // meta.visuals()

    // Find the default audio track
    let track = probed.format.default_track().ok_or_else(|| ParseError::TrackNotFound)?;

    info!("Track found, audio codec: {:?}", track.codec_params);

    /*result.bits_per_sample =
        track.codec_params.bits_per_sample.ok_or_else(|| ParseError::MetadataNotFound("bits_per_sample".to_string()))?;*/
    result.format = get_format_str(track.codec_params.codec).ok_or_else(|| ParseError::FormatUnsupported)?.to_string();
    // Calculate duration
    result.duration_secs = calculate_duration_secs(&track)?.ok_or_else(|| ParseError::ParsingDurationError)?;
    result.sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let (gain, peak) = calculate_gain_peak(&mut probed.format)
        .map_err(|x| {
            warn!("Failed to calculate gain/peak: {x:?}");
            ParseError::CalculatingGainPeakError
        })?;
    result.gain_db = gain;
    result.peak = peak;
    Ok(result)
}

fn get_format_str(codec_type: CodecType) -> Option<&'static str> {
    match codec_type {
        codecs::CODEC_TYPE_MP3 => Some("mp3"),
        codecs::CODEC_TYPE_AAC => Some("aac"),
        codecs::CODEC_TYPE_FLAC => Some("flac"),
        _ => None
    }
}
fn calculate_duration_secs(track: &Track) -> Result<Option<u64>, ParseError> {
    let r = if let Some(tb) = track.codec_params.time_base {
        let frames = track.codec_params.n_frames.ok_or_else(|| ParseError::ParsingDurationError)?;
        let duration = tb.calc_time(frames);
        Some(duration.seconds)
    } else {
        None
    };
    Ok(r)
}

fn calculate_gain_peak(format: &mut Box<dyn FormatReader>) -> anyhow::Result<(f32, f32)> {
    let (spec, samples) = read_interleaved_samples(format)?;
    let mut rg = ReplayGain::new(spec.rate as usize).unwrap();
    rg.process_samples(&samples);
    let (gain, peak) = rg.finish();
    Ok((gain, peak))
}

fn read_interleaved_samples(format: &mut Box<dyn FormatReader>) -> anyhow::Result<(SignalSpec, Vec<f32>)> {
    let track = format.default_track().ok_or_else(|| anyhow!("Can't get default track"))?;
    let track_id = track.id;
    let codec = symphonia::default::get_codecs();
    let mut decoder = codec.make(&track.codec_params, &DecoderOptions::default())
        .with_context(|| "Can't create decoder")?;

    let mut sample_count = 0;
    let mut sample_buf = None;
    let mut spec = None;

    let mut samples: Vec<f32> =  Vec::new();
    loop {
        // Get the next packet from the format reader.
        let packet = match format.next_packet() {
            Ok(x) => { x }
            Err(err) => {
                break
            }
        };

        // If the packet does not belong to the selected track, skip it.
        if packet.track_id() != track_id {
            continue;
        }

        // Decode the packet into audio samples, ignoring any decode errors.
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                // The decoded audio samples may now be accessed via the audio buffer if per-channel
                // slices of samples in their native decoded format is desired. Use-cases where
                // the samples need to be accessed in an interleaved order or converted into
                // another sample format, or a byte buffer is required, are covered by copying the
                // audio buffer into a sample buffer or raw sample buffer, respectively. In the
                // example below, we will copy the audio buffer into a sample buffer in an
                // interleaved order while also converting to a f32 sample format.

                // If this is the *first* decoded packet, create a sample buffer matching the
                // decoded audio buffer format.
                if sample_buf.is_none() {
                    // Get the audio buffer specification.
                    spec = Some(*audio_buf.spec());

                    // Get the capacity of the decoded buffer. Note: This is capacity, not length!
                    let duration = audio_buf.capacity() as u64;

                    // Create the f32 sample buffer.
                    sample_buf = Some(SampleBuffer::<f32>::new(duration, spec.unwrap()));
                }

                // Copy the decoded audio buffer into the sample buffer in an interleaved format.
                if let Some(buf) = &mut sample_buf {
                    buf.copy_interleaved_ref(audio_buf);
                    // Append buf.samples to `samples``
                    samples.extend(buf.samples());
                    // The samples may now be access via the `samples()` function.
                    sample_count += buf.samples().len();
                }
            }
            Err(SymphoniaError::DecodeError(_)) => (),
            Err(_) => break,
        }
    }

    let spec = spec.ok_or_else(|| anyhow!("Skipped all samples"))?;
    Ok((spec, samples))
}

#[cfg(test)]
mod tests {
    use crate::audio::parse_and_validate;
    use std::fs;

    #[test]
    fn test_parse() {
        let file = fs::File::open(".local/test.mp3").unwrap();
        let result = parse_and_validate(Box::new(file), Some("test.mp3")).unwrap();
        println!("{:?}", result);
    }
}
