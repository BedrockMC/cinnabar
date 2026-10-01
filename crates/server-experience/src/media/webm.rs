//! Developer-only AV1/Opus pipeline. Native codecs need a restricted helper before release.

use super::{
    descriptor::Descriptor,
    frames::{PcmBlock, VideoFrame, bt709_rgba},
    ranges::RangeReader,
    *,
};
use anyhow::{Result, ensure};
use dav1d::{Decoder, PixelLayout, PlanarImageComponent, Settings};
use matroska_demuxer::{FlagInterlaced, Frame, MatroskaFile, TrackType};
use std::{
    collections::BTreeSet,
    io::{Read, Seek},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

const CICP_BT709: u8 = 1;
static DECODER_ACTIVE: AtomicBool = AtomicBool::new(false);

struct DecoderLease;

impl Drop for DecoderLease {
    /// Holds the process-wide decoder slot until the worker really exits.
    fn drop(&mut self) {
        DECODER_ACTIVE.store(false, Ordering::Release);
    }
}

pub use super::service::output::Output;

pub struct Worker {
    cancelled: Arc<AtomicBool>,
    output: Mutex<mpsc::Receiver<Result<Output>>>,
}

impl Worker {
    /// A cancelled worker retains the decoder slot until it has actually exited.
    pub fn available() -> bool {
        !DECODER_ACTIVE.load(Ordering::Acquire)
    }

    /// Starts fetching and decoding off-thread only under the explicit developer switch.
    pub fn start(
        descriptor: Descriptor,
        origins: BTreeSet<String>,
        generation: u64,
        data_budget: Arc<AtomicU64>,
        start_us: u64,
    ) -> Result<Self> {
        ensure!(
            std::env::var(crate::policy::DEVELOPER_ENV).as_deref() == Ok("1"),
            "native media requires a restricted production helper"
        );
        descriptor.validate(&origins)?;
        ensure!(
            DECODER_ACTIVE
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "media decoder already active"
        );
        let lease = DecoderLease;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let (sender, output) = mpsc::sync_channel(MAX_FRAMES);
        std::thread::Builder::new()
            .name("experience-media".into())
            .spawn(move || {
                let _lease = lease;
                let result = (|| -> Result<()> {
                    let reader = RangeReader::new(
                        descriptor.clone(),
                        origins,
                        Arc::clone(&stop),
                        data_budget,
                    )?;
                    decode(reader, &descriptor, generation, &stop, |frame| {
                        match &frame {
                            Output::Video(frame) if frame.pts_us < start_us => return Ok(()),
                            Output::Audio(block) if block.pts_us < start_us => return Ok(()),
                            _ => {}
                        }
                        sender
                            .send(Ok(frame))
                            .map_err(|_| anyhow::anyhow!("media consumer closed"))
                    })
                })();
                if let Err(error) = result {
                    let _ = sender.send(Err(error));
                }
            })?;
        Ok(Self {
            cancelled,
            output: Mutex::new(output),
        })
    }

    /// Reads only completed output; frame validation is repeated by the consumer.
    pub fn poll(&self) -> Option<Result<Output>> {
        self.output.lock().ok()?.try_recv().ok()
    }
}

impl Drop for Worker {
    /// Cancels network reads; dropping the receiver also releases a blocked producer.
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Decodes one constrained rendition; seeking restarts this worker with a fresh generation.
fn decode<R: Read + Seek>(
    reader: R,
    descriptor: &Descriptor,
    generation: u64,
    cancelled: &AtomicBool,
    mut emit: impl FnMut(Output) -> Result<()>,
) -> Result<()> {
    let mut file = MatroskaFile::open(reader)?;
    ensure!(
        file.ebml_header().doc_type() == "webm" && file.tracks().len() == 2,
        "unsupported WebM structure"
    );
    ensure!(
        file.chapters().is_none() && file.tags().is_none(),
        "chapters and tags are outside the media profile"
    );
    let video = file
        .tracks()
        .iter()
        .find(|track| track.track_type() == TrackType::Video)
        .ok_or_else(|| anyhow::anyhow!("missing video track"))?;
    let audio = file
        .tracks()
        .iter()
        .find(|track| track.track_type() == TrackType::Audio)
        .ok_or_else(|| anyhow::anyhow!("missing audio track"))?;
    ensure!(
        video.codec_id() == "V_AV1" && audio.codec_id() == "A_OPUS",
        "unsupported codec"
    );
    ensure!(
        file.tracks()
            .iter()
            .all(|track| track.content_encodings().is_none() && !track.flag_lacing()),
        "track transformations or lacing denied"
    );
    let geometry = video
        .video()
        .ok_or_else(|| anyhow::anyhow!("missing video metadata"))?;
    ensure!(
        geometry.pixel_width().get() == u64::from(descriptor.width)
            && geometry.pixel_height().get() == u64::from(descriptor.height)
            && geometry.flag_interlaced() == FlagInterlaced::Progressive
            && geometry.alpha_mode().unwrap_or(0) == 0,
        "video profile mismatch"
    );
    let format = audio
        .audio()
        .ok_or_else(|| anyhow::anyhow!("missing audio metadata"))?;
    ensure!(
        format.channels().get() == u64::from(descriptor.audio_channels)
            && format.sampling_frequency() == f64::from(SAMPLE_RATE),
        "audio profile mismatch"
    );
    let header = audio
        .codec_private()
        .ok_or_else(|| anyhow::anyhow!("missing OpusHead"))?;
    ensure!(
        header.len() == 19
            && &header[..8] == b"OpusHead"
            && header[8] == 1
            && header[9] == descriptor.audio_channels
            && header[18] == 0,
        "unsupported OpusHead"
    );
    let mut skip = usize::from(u16::from_le_bytes([header[10], header[11]]));
    ensure!(skip <= SAMPLE_RATE as usize, "Opus pre-skip too large");
    let delay_ns = audio.codec_delay().unwrap_or(0);
    let expected_delay = skip as u64 * 1_000_000_000 / u64::from(SAMPLE_RATE);
    ensure!(
        delay_ns.abs_diff(expected_delay) <= 1,
        "Opus delay mismatch"
    );
    let video_track = video.track_number().get();
    let audio_track = audio.track_number().get();
    let scale = file.info().timestamp_scale().get();
    let mut settings = Settings::new();
    settings.set_n_threads(2);
    settings.set_max_frame_delay(1);
    settings.set_frame_size_limit(MAX_WIDTH * MAX_HEIGHT);
    settings.set_strict_std_compliance(true);
    let mut av1 = Decoder::with_settings(&settings)?;
    let channels = if descriptor.audio_channels == 1 {
        opus::Channels::Mono
    } else {
        opus::Channels::Stereo
    };
    let mut opus = opus::Decoder::new(SAMPLE_RATE, channels)?;
    opus.set_gain(i32::from(i16::from_le_bytes([header[16], header[17]])))?;
    let mut frame = Frame::default();
    let mut last_video_us = None;
    while file.next_frame(&mut frame)? {
        ensure!(!cancelled.load(Ordering::Acquire), "media cancelled");
        ensure!(
            frame.data.len() <= MAX_SAMPLE_BYTES,
            "compressed sample too large"
        );
        let pts_us = frame
            .timestamp
            .checked_mul(scale)
            .ok_or_else(|| anyhow::anyhow!("timestamp overflow"))?
            / 1000;
        ensure!(
            pts_us <= descriptor.duration_us.saturating_add(1_000_000),
            "sample exceeds duration"
        );
        if frame.track == video_track {
            if let Some(last) = last_video_us {
                ensure!(
                    pts_us > last
                        && pts_us - last >= (1_000_000 / u64::from(MAX_FPS)).saturating_sub(1000),
                    "decoded frame rate exceeded"
                );
            }
            last_video_us = Some(pts_us);
            let mut result = av1.send_data(
                std::mem::take(&mut frame.data),
                None,
                Some(i64::try_from(pts_us)?),
                None,
            );
            for _ in 0..64 {
                match result {
                    Ok(()) => break,
                    Err(dav1d::Error::Again) => {
                        drain(&mut av1, descriptor, generation, &mut emit)?;
                        result = av1.send_pending_data();
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            result?;
            drain(&mut av1, descriptor, generation, &mut emit)?;
        } else if frame.track == audio_track {
            let channels = usize::from(descriptor.audio_channels);
            let mut samples = vec![0.0; OPUS_PACKET_FRAMES * channels];
            let count = opus.decode_float(&frame.data, &mut samples, false)?;
            ensure!(count <= OPUS_PACKET_FRAMES, "Opus packet duration exceeded");
            samples.truncate(count * channels);
            clamp_opus(&mut samples)?;
            let skipped = skip.min(count);
            skip -= skipped;
            samples.drain(..skipped * channels);
            let corrected = i128::from(pts_us) - i128::from(delay_ns / 1000)
                + (skipped as i128 * 1_000_000 / i128::from(SAMPLE_RATE));
            if !samples.is_empty() {
                let block = PcmBlock {
                    generation,
                    pts_us: u64::try_from(corrected.max(0))?,
                    channels: descriptor.audio_channels,
                    samples,
                };
                block.validate(generation)?;
                emit(Output::Audio(block))?;
            }
        } else {
            anyhow::bail!("unexpected track");
        }
    }
    drain(&mut av1, descriptor, generation, &mut emit)?;
    emit(Output::End)
}

/// Copies validated dav1d planes into one bounded frame; invisible/reference frames stay internal.
fn drain(
    decoder: &mut Decoder,
    descriptor: &Descriptor,
    generation: u64,
    emit: &mut impl FnMut(Output) -> Result<()>,
) -> Result<()> {
    for _ in 0..64 {
        let picture = match decoder.get_picture() {
            Ok(picture) => picture,
            Err(dav1d::Error::Again) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            picture.width() == descriptor.width
                && picture.height() == descriptor.height
                && picture.bit_depth() == 8
                && picture.pixel_layout() == PixelLayout::I420,
            "decoded profile changed"
        );
        ensure!(
            picture.matrix_coefficients() as u8 == CICP_BT709
                && picture.color_primaries() as u8 == CICP_BT709
                && picture.transfer_characteristic() as u8 == CICP_BT709
                && picture.color_range() == dav1d::pixel::YUVRange::Limited,
            "unsupported decoded color profile"
        );
        let components = [
            PlanarImageComponent::Y,
            PlanarImageComponent::U,
            PlanarImageComponent::V,
        ];
        let planes = components.map(|component| picture.plane(component));
        let strides = components.map(|component| picture.stride(component) as usize);
        let rgba = bt709_rgba(
            picture.width(),
            picture.height(),
            [&planes[0], &planes[1], &planes[2]],
            strides,
        )?;
        let pts_us = u64::try_from(
            picture
                .timestamp()
                .ok_or_else(|| anyhow::anyhow!("missing video PTS"))?,
        )?;
        let frame = VideoFrame {
            generation,
            pts_us,
            width: picture.width(),
            height: picture.height(),
            rgba,
        };
        frame.validate(generation)?;
        emit(Output::Video(frame))?;
    }
    anyhow::bail!("too many decoded frames in one dispatch")
}

/// Preserves finite decoder headroom and positive OpusHead gain without rejecting loud audio.
fn clamp_opus(samples: &mut [f32]) -> Result<()> {
    for sample in samples {
        ensure!(sample.is_finite(), "nonfinite Opus output");
        *sample = sample.clamp(-1.0, 1.0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loud_opus_with_positive_header_gain_is_bounded_after_decode() {
        let channels = opus::Channels::Mono;
        let mut encoder =
            opus::Encoder::new(SAMPLE_RATE, channels, opus::Application::Audio).unwrap();
        let source: Vec<_> = (0..OPUS_PACKET_FRAMES)
            .map(|i| (i as f32 * 0.1).sin() * 0.95)
            .collect();
        let mut packet = vec![0; 4096];
        let len = encoder.encode_float(&source, &mut packet).unwrap();
        let mut decoder = opus::Decoder::new(SAMPLE_RATE, channels).unwrap();
        decoder.set_gain(12 * 256).unwrap();
        let mut samples = vec![0.0; OPUS_PACKET_FRAMES];
        let count = decoder
            .decode_float(&packet[..len], &mut samples, false)
            .unwrap();
        samples.truncate(count);
        assert!(samples.iter().any(|sample| sample.abs() > 1.0));
        clamp_opus(&mut samples).unwrap();
        PcmBlock {
            generation: 1,
            pts_us: 0,
            channels: 1,
            samples,
        }
        .validate(1)
        .unwrap();
        assert!(clamp_opus(&mut [f32::NAN]).is_err());
        assert!(clamp_opus(&mut [f32::INFINITY]).is_err());
    }
}
