use std::convert::TryInto;
use std::io::Read;

use ffmpeg_next::{
    format::{input, Pixel},
    media::Type as MediaType,
    software::scaling::{context::Context, Flags as ScalingFlags},
    util::frame::Video,
};
use image::{gif::GifDecoder, AnimationDecoder};
use tempfile::NamedTempFile;

use crate::get_hasher;

/// Extract frames of a GIF into individual images and calculate a hash for each
/// frame. Results are kept in the same order as seen in the GIF.
///
/// This is a blocking function.
#[tracing::instrument(skip(r))]
pub fn extract_gif_hashes<R: Read>(r: R) -> Result<Vec<[u8; 8]>, image::ImageError> {
    let hasher = crate::get_hasher();

    // Begin by creating a new GifDecoder from our reader. Collect all frames
    // from the GIF.
    //
    // FUTURE: profile memory usage of collecting all frames instead of iterating
    let decoder = GifDecoder::new(r)?;
    let frames = decoder.into_frames().collect_frames()?;

    tracing::trace!(frames = frames.len(), "Collected GIF frames");

    // Allocate a Vec to hold all our hashes.
    let mut hashes = Vec::with_capacity(frames.len());

    // For each frame, get an ImageBuffer, hash the image, and append bytes into
    // the results.
    //
    // FUTURE: should this be parallelized?
    for frame in frames {
        let buf = frame.buffer();

        let hash = hasher.hash_image(buf);
        let bytes = hash.as_bytes().try_into().unwrap();

        hashes.push(bytes);
    }

    Ok(hashes)
}

/// Write the contents of `r` into a temporary file and return the handle to
/// that file. This file should automatically be deleted when the handle is
/// dropped.
///
/// This is a blocking function.
fn write_temp_file<R: Read>(mut r: R) -> std::io::Result<NamedTempFile> {
    let mut f = NamedTempFile::new()?;
    std::io::copy(&mut r, &mut f)?;

    Ok(f)
}

/// Extract frames of a video into individual images and calculate a hash for
/// each frame. Results are kept in the same order as seen in the input.
///
/// This is a blocking function.
#[tracing::instrument(skip(r))]
pub fn extract_video_hashes<R: Read>(r: R) -> anyhow::Result<Vec<[u8; 8]>> {
    let f = write_temp_file(r)?;

    // Create an input context from the given path.
    //
    // TODO: figure out if there's a way to provide data without creating a file
    let mut ictx = input(&f.path())?;

    // Select the best video stream and find it's index.
    let input = ictx
        .streams()
        .best(MediaType::Video)
        .ok_or(ffmpeg_next::Error::StreamNotFound)?;
    let stream_index = input.index();

    // Create a new decoder that outputs 8-bit RGB colors with the same
    // dimensions as the source.
    let mut decoder = input.codec().decoder().video()?;
    let mut scaler = Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        Pixel::RGB24,
        decoder.width(),
        decoder.height(),
        ScalingFlags::BILINEAR,
    )?;

    tracing::trace!("Initialized ffmpeg with video input");

    let mut hashes: Vec<[u8; 8]> = Vec::new();
    let hasher = get_hasher();

    // Callback function run for each packet loaded by ffmpeg. It's responsible
    // for processing each frame into a hash and storing it.
    let mut receive_and_process_decoded_frames =
        |decoder: &mut ffmpeg_next::decoder::Video| -> Result<(), ffmpeg_next::Error> {
            let mut decoded = Video::empty();

            while decoder.receive_frame(&mut decoded).is_ok() {
                // Create a frame buffer and decode data into it.
                let mut rgb_frame = Video::empty();
                scaler.run(&decoded, &mut rgb_frame)?;

                // Convert raw data into an RgbImage for use with image hashing.
                let data = rgb_frame.data(0).to_vec();
                let im: image::RgbImage =
                    image::ImageBuffer::from_raw(decoder.width(), decoder.height(), data)
                        .expect("Image frame data was invalid");

                // Hash frame, convert to [u8; 8].
                let hash = hasher.hash_image(&im);
                let hash = hash.as_bytes();
                hashes.push(
                    hash.try_into()
                        .expect("img_hash provided incorrect number of bytes"),
                );
            }

            Ok(())
        };

    // Now that we've set up our callback, iterate through file packets, decode
    // them, and send to our callback for processing.
    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        decoder.send_packet(&packet)?;
        receive_and_process_decoded_frames(&mut decoder)?;
    }

    // Make sure all data has been processed with EOF.
    decoder.send_eof()?;
    receive_and_process_decoded_frames(&mut decoder)?;

    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_gif_hashes() -> anyhow::Result<()> {
        use std::fs::File;

        let gif = File::open("tests/fox.gif")?;
        let hashes = extract_gif_hashes(&gif)?;

        assert_eq!(
            hashes.len(),
            47,
            "GIF did not have expected number of hashes"
        );

        assert_eq!(
            hashes[0],
            [154, 64, 160, 169, 170, 53, 181, 221],
            "First frame had different hash"
        );
        assert_eq!(
            hashes[1],
            [154, 64, 160, 169, 170, 53, 53, 221],
            "Second frame had different hash"
        );

        Ok(())
    }

    #[test]
    fn test_extract_video_hashes() -> anyhow::Result<()> {
        use std::fs::File;

        let video = File::open("tests/video.webm")?;
        let hashes = extract_video_hashes(&video)?;

        assert_eq!(
            hashes.len(),
            126,
            "Video did not have expected number of hashes"
        );

        assert_eq!(
            hashes[0],
            [60, 166, 75, 61, 48, 166, 73, 205],
            "First frame had different hash"
        );
        assert_eq!(
            hashes[1],
            [60, 166, 75, 61, 48, 166, 73, 205],
            "Second frame had different hash"
        );

        Ok(())
    }
}
