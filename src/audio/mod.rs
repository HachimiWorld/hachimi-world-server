use anyhow::{anyhow, Context};
use replaygain::ReplayGain;
use symphonia::core::audio::sample::Sample;
use symphonia::core::audio::AudioSpec;
use symphonia::core::codecs;
use symphonia::core::codecs::audio::{AudioCodecId, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, Track, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, StandardTag};
use symphonia::core::units::Timestamp;
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
    pub gain_db: f32,
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

    // Init the media source stream
    let media = MediaSourceStream::new(input, MediaSourceStreamOptions::default());

    // Use file extension as hint if possible
    let mut hint = Hint::new();
    if let Some((_, ext)) = file_name.and_then(|name| name.rsplit_once(".")) {
        debug!("File extension hint: {ext}");
        hint.with_extension(ext);
    } else {
        debug!("No hint");
    }

    // Use default format options and metadata options to probe the format
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    // Probe the media source stream for a format.
    let mut format = symphonia::default::get_probe()
        .probe(&hint, media, fmt_opts, meta_opts)
        .map_err(|x| ParseError::Parse(x))?;

    // Retrieve the first metadata found in the format\
    let metadata = (&mut format).metadata();
    let metadata = metadata.current()
        .ok_or_else(|| ParseError::MetadataNotFound("root".to_string()))?;


    // Retrieve metadata
    for tag in &metadata.media.tags {
        if let Some(key) = tag.std.to_owned() {
            match key {
                StandardTag::TrackTitle(value) => result.title = Some(value.to_string()),
                StandardTag::Artist(value) => result.artist = Some(value.to_string()),
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
    let track = format.default_track(TrackType::Audio).ok_or_else(|| ParseError::TrackNotFound)?;

    info!("Track found, audio codec: {:?}", track.codec_params);
    let codec_params = if let Some(codec) = &track.codec_params &&
        let Some(audio_codec) = codec.audio() {
        audio_codec
    } else {
        Err(ParseError::FormatUnsupported)?
    };


    /*result.bits_per_sample =
        track.codec_params.bits_per_sample.ok_or_else(|| ParseError::MetadataNotFound("bits_per_sample".to_string()))?;*/
    result.format = get_format_str(codec_params.codec).ok_or_else(|| ParseError::FormatUnsupported)?.to_string();
    // Calculate duration
    result.duration_secs = calculate_duration_secs(&track)?.ok_or_else(|| ParseError::ParsingDurationError)?;
    result.sample_rate = codec_params.sample_rate.unwrap_or(0);
    let (gain, peak) = calculate_gain_peak(&mut format)
        .map_err(|x| {
            warn!("Failed to calculate gain/peak: {x:?}");
            ParseError::CalculatingGainPeakError
        })?;
    result.gain_db = gain;
    result.peak = peak;
    Ok(result)
}

fn get_format_str(codec_type: AudioCodecId) -> Option<&'static str> {
    match codec_type {
        codecs::audio::well_known::CODEC_ID_MP3 => Some("mp3"),
        codecs::audio::well_known::CODEC_ID_AAC => Some("aac"),
        codecs::audio::well_known::CODEC_ID_FLAC => Some("flac"),
        _ => None
    }
}

fn calculate_duration_secs(track: &Track) -> Result<Option<u64>, ParseError> {
    let r = if let Some(tb) = track.time_base {
        let frames = track.num_frames.ok_or_else(|| ParseError::ParsingDurationError)?;
        let duration = tb.calc_time(Timestamp::new(frames as i64)).ok_or_else(|| ParseError::ParsingDurationError)?;
        Some(duration.as_secs() as u64)
    } else {
        None
    };
    Ok(r)
}

fn calculate_gain_peak(format: &mut Box<dyn FormatReader>) -> anyhow::Result<(f32, f32)> {
    let (spec, samples) = read_interleaved_samples(format)?;
    let mut rg = ReplayGain::new(spec.rate() as usize)
        .ok_or_else(|| anyhow!("This sample rate is not supported: {}", spec.rate()))?;
    rg.process_samples(&samples);
    let (gain, peak) = rg.finish();
    Ok((gain, peak))
}

fn read_interleaved_samples(format: &mut Box<dyn FormatReader>) -> anyhow::Result<(AudioSpec, Vec<f32>)> {
    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("Can't get default track"))?;
    let track_id = track.id;
    let codec_params = if let Some(codec) = &track.codec_params &&
        let Some(audio_codec) = codec.audio() {
        audio_codec
    } else {
        Err(ParseError::FormatUnsupported)?
    };
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
        .with_context(|| "Can't create decoder")?;

    let mut spec = None;
    let mut samples: Vec<f32> = Vec::new();
    let mut samples_buf: Vec<f32> = Vec::new();
    let mut _total_sample_count = 0;

    while let Ok(Some(packet)) = format.next_packet() {
        // If the packet does not belong to the selected track, skip it.
        if packet.track_id != track_id {
            continue;
        }

        // Decode the packet into audio samples, ignoring any decode errors.
        // Copy all the samples into a vector in the f32 sample format in channel interleaved order.
        match decoder.decode(&packet) {
            Ok(audio_buf) => {
                // If this is the *first* decoded packet, create a sample buffer matching the
                // decoded audio buffer format.
                if spec.is_none() {
                    // Get the audio buffer specification.
                    spec = Some(audio_buf.spec().to_owned());
                }

                // Ensure the vector is large enough to hold all the samples.
                samples_buf.resize(audio_buf.samples_interleaved(), f32::MID);
                // Copy the audio samples from the generic audio buffer to the vector in interleaved
                // order. The sample format to convert to is inferred from the type of the Vec.
                audio_buf.copy_to_slice_interleaved(&mut samples_buf);

                _total_sample_count += samples_buf.len();
                samples.extend(&samples_buf);
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
    fn test_parse_mp3() {
        let file = fs::File::open("tests/fixtures/test-mp3.mp3").unwrap();
        let result = parse_and_validate(Box::new(file), Some("test-mp3.mp3")).unwrap();
        assert_eq!(result.format, "mp3");
        assert_eq!(result.title, Some("Test Track".to_string()));
        assert_eq!(result.artist, Some("Test Artist".to_string()));
        assert_eq!(result.duration_secs, 10);
        assert_eq!(result.sample_rate, 44100);
        assert_ne!(result.peak, 0f32);
        assert_ne!(result.gain_db, 0f32);
    }
}
