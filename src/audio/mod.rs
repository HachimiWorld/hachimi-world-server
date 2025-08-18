use symphonia::core::codecs;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;
use thiserror::Error;

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
}

pub fn parse_and_validate(
    input: Box<dyn MediaSource>,
    file_name: Option<&str>,
) -> Result<PickedMetadata, ParseError> {
    let file_name = file_name.and_then(|name| name.rsplit_once("."));

    let media = MediaSourceStream::new(input, Default::default());
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let mut hint = Hint::new();
    if let Some((_, ext)) = file_name {
        hint.with_extension(ext);
    }

    let mut result = PickedMetadata {
        format: "mp3".to_string(),
        title: None,
        artist: None,
        bitrate: 0,
        bit_depth: 0,
        sample_rate: 0,
        duration_secs: 0,
    };

    let mut probed = symphonia::default::get_probe()
        .format(&hint, media, &fmt_opts, &meta_opts)
        .map_err(|x| ParseError::Parse(x))?;

    // let probed_metadata = probe.metadata.get();
    // let format_metadata = probe.format.metadata();
    //
    // let best = format_metadata.current();
    // let choice2 = probed_metadata.and_then(|x| x.current());
    let metadata = probed.format
        .metadata()
        .current()
        .map(|x| x.to_owned())
        .or_else(|| {
            probed
                .metadata
                .get()
                .and_then(|m| m.current().map(|r| r.to_owned()))
        });

    if let Some(meta) = metadata {
        // So fucking complex api
        // Retrieve metadata
        for tag in meta.tags() {
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
    } else {
        Err(ParseError::MetadataNotFound("root".to_string()))?
    }

    // Find the first audio track
    let track = probed
        .format
        .default_track()
        .ok_or_else(|| ParseError::TrackNotFound)?;

    /*result.bits_per_sample = track
        .codec_params
        .bits_per_sample
        .ok_or_else(|| ParseError::MetadataNotFound("bits_per_sample".to_string()))?;
*/
    let codec_type = track.codec_params.codec;

    let format = match codec_type {
        codecs::CODEC_TYPE_MP3 => "mp3",
        codecs::CODEC_TYPE_AAC => "aac",
        codecs::CODEC_TYPE_FLAC => "flac",
        _ => Err(ParseError::FormatUnsupported)?
    };

    result.format = format.to_string();

    // Calculate duration
    if let Some(tb) = track.codec_params.time_base {
        let frames = track.codec_params.n_frames.ok_or_else(|| ParseError::ParsingDurationError)?;
        let duration = tb.calc_time(frames);
        result.duration_secs = duration.seconds;
    };
    
    result.sample_rate = track.codec_params.sample_rate.unwrap_or(0);

    // Fuck, I will do it later
    Ok(result)
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
