//! Decoding tests against small files made with the `ffmpeg` command-line tool (skipped,
//! with a note, when it isn't installed).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use jd_media::{Player, VideoDecoder, probe, thumbnail};

/// A 4 s, 25 fps, 320×240 H.264 clip with a keyframe every 25 frames, plus a stereo tone.
fn fixture() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("jd-media-tests");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("testsrc_25fps.mp4");
    if path.exists() {
        return Some(path);
    }
    let tmp = dir.join(format!("tmp-{}.mp4", std::process::id()));
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=4:size=320x240:rate=25"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4:sample_rate=48000"])
        .args(["-ac", "2", "-c:v", "libx264", "-g", "25", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&tmp)
        .status();
    match status {
        Ok(s) if s.success() => {
            std::fs::rename(&tmp, &path).ok()?;
            Some(path)
        }
        _ => {
            eprintln!("skipping: couldn't make a test video with the ffmpeg CLI");
            None
        }
    }
}

fn frame_index(t: f64) -> i64 {
    (t * 25.0).round() as i64
}

#[test]
fn probe_reports_streams_and_duration() {
    let Some(path) = fixture() else { return };
    let info = probe(&path).unwrap();
    assert!((info.duration - 4.0).abs() < 0.1, "duration {}", info.duration);
    let v = info.video.unwrap();
    assert_eq!((v.width, v.height, v.rate.rounded(), v.still), (320, 240, 25, false));
    assert_eq!(v.codec, "h264");
    let a = info.audio.unwrap();
    assert_eq!((a.sample_rate, a.channels), (48_000, 2));
}

#[test]
fn seeking_is_frame_accurate_between_keyframes() {
    let Some(path) = fixture() else { return };
    let mut d = VideoDecoder::open(&path, (640, 640)).unwrap();
    // Frames 0, 13 (mid-GOP), 37, 99 (last), backwards too.
    for &frame in &[37, 13, 0, 99, 50, 49] {
        let t = frame as f64 / 25.0;
        let got = d.seek(t).unwrap().unwrap();
        assert_eq!(frame_index(got), frame, "asked for frame {frame}, got {got}s");
    }
    // Past the end: the last frame.
    assert_eq!(frame_index(d.seek(10.0).unwrap().unwrap()), 99);
}

#[test]
fn frames_decode_in_order_after_a_seek() {
    let Some(path) = fixture() else { return };
    let mut d = VideoDecoder::open(&path, (640, 640)).unwrap();
    d.seek(1.0).unwrap();
    let next: Vec<i64> = (0..5).map(|_| frame_index(d.next_time().unwrap().unwrap())).collect();
    assert_eq!(next, [26, 27, 28, 29, 30]);
}

#[test]
fn frames_are_rgba_scaled_to_fit() {
    let Some(path) = fixture() else { return };
    let f = thumbnail(&path, 2.0, (160, 160)).unwrap();
    assert_eq!((f.width, f.height), (160, 120));
    assert_eq!(f.rgba.len(), 160 * 120 * 4);
    assert!(f.rgba.chunks(4).all(|p| p[3] == 255), "video is opaque");
    // testsrc is colourful, not a blank frame.
    let distinct: std::collections::HashSet<_> = f.rgba.chunks(4).map(|p| (p[0] / 32, p[1] / 32, p[2] / 32)).collect();
    assert!(distinct.len() > 5);
}

fn wait_until(player: &Player, what: &str, ok: impl Fn(&jd_media::Status) -> bool) {
    let start = Instant::now();
    while !(ok(&player.status()) && !player.busy()) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}: {:?}", player.status());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn player_loads_scrubs_steps_and_plays_to_the_end() {
    let Some(path) = fixture() else { return };
    let player = Player::new((320, 320));
    player.load(path.clone());
    wait_until(&player, "load", |s| s.path.as_deref() == Some(path.as_path()));
    assert!(player.take_frame().is_some(), "first frame shown after load");

    // Scrub: many seeks, the last one wins.
    for i in 0..30 {
        player.seek(i as f64 * 0.1);
    }
    wait_until(&player, "scrub", |s| frame_index(s.position) == 73);
    assert_eq!(frame_index(player.take_frame().unwrap().time), 73);

    player.step(2);
    wait_until(&player, "step", |s| frame_index(s.position) == 75);
    player.step(-10);
    wait_until(&player, "step back", |s| frame_index(s.position) == 65);

    // Play at 4x from 3.0 s: reaches the last frame in about 0.25 s and stops.
    player.seek(3.0);
    player.play(4.0);
    wait_until(&player, "play to end", |s| !s.playing && frame_index(s.position) == 99);
    assert!(player.status().error.is_none());
}

#[test]
fn missing_file_reports_an_error() {
    let player = Player::new((64, 64));
    player.load(Path::new("/nonexistent/clip.mp4").to_path_buf());
    wait_until(&player, "error", |s| s.error.is_some());
    assert!(probe(Path::new("/nonexistent/clip.mp4")).is_err());
}

#[test]
fn audio_decodes_to_stereo_at_the_asked_rate() {
    let Some(path) = fixture() else { return };
    let mut d = jd_media::AudioDecoder::open(&path, 48_000).unwrap();
    let mut buf = vec![0.0f32; 48_000 * 2];
    let mut total = 0;
    let mut first_half_second = Vec::new();
    loop {
        let n = d.read(&mut buf).unwrap();
        if first_half_second.is_empty() {
            first_half_second = buf[..24_000 * 2].to_vec();
        }
        total += n;
        if n < 48_000 {
            break;
        }
    }
    // 4 s, give or take the AAC encoder's padding.
    assert!((total as i64 - 4 * 48_000).abs() < 3_000, "{total} frames");
    // A 440 Hz sine: about 440 zero crossings (up and down) in half a second, on the left.
    let left: Vec<f32> = first_half_second.iter().step_by(2).copied().collect();
    let crossings = left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    assert!((430..=450).contains(&crossings), "{crossings} crossings");
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
}

#[test]
fn audio_seek_starts_reading_at_the_exact_time() {
    let Some(path) = fixture() else { return };
    let mut d = jd_media::AudioDecoder::open(&path, 48_000).unwrap();
    d.seek(2.5).unwrap();
    let mut buf = vec![0.0f32; 4 * 48_000 * 2];
    let n = d.read(&mut buf).unwrap();
    assert!((n as i64 - 72_000).abs() < 3_000, "1.5 s left after 2.5 s, got {n} frames");

    // Reading from 1.0 s matches reading everything and skipping to 1.0 s.
    let mut whole = jd_media::AudioDecoder::open(&path, 48_000).unwrap();
    let mut all = vec![0.0f32; 4 * 48_000 * 2];
    whole.read(&mut all).unwrap();
    d.seek(1.0).unwrap();
    let mut part = vec![0.0f32; 2_000];
    d.read(&mut part).unwrap();
    let expected = &all[48_000 * 2..48_000 * 2 + 2_000];
    let err = part.iter().zip(expected).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(err < 0.02, "seeked samples differ by {err}");
}

#[test]
fn waveform_has_levels_per_bucket() {
    let Some(path) = fixture() else { return };
    let w = jd_media::waveform(&path, 10.0).unwrap();
    assert!((38..=42).contains(&w.len()), "{} buckets", w.len());
    assert!(w.iter().all(|&p| (0.0..=1.0).contains(&p)));
    assert!(w[5] > 0.05, "the tone is audible");
}

/// Runs the ffmpeg CLI into `dir/name` once; None if that fails.
fn make(name: &str, args: &[&str]) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("jd-media-tests");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    if path.exists() {
        return Some(path);
    }
    let tmp = dir.join(format!("tmp-{}-{name}", std::process::id()));
    let ok = Command::new("ffmpeg").args(["-y", "-loglevel", "error"]).args(args).arg(&tmp).status().is_ok_and(|s| s.success());
    if !ok {
        eprintln!("skipping: ffmpeg couldn't make {name}");
        return None;
    }
    std::fs::rename(&tmp, &path).ok()?;
    Some(path)
}

#[test]
fn portrait_phone_video_is_shown_upright() {
    let Some(src) = fixture() else { return };
    // Like a phone: stored landscape, with a display matrix saying "turn it 90°".
    let Some(path) = make("rotated.mp4", &["-display_rotation", "90", "-i", src.to_str().unwrap(), "-c", "copy"]) else { return };
    let v = probe(&path).unwrap().video.unwrap();
    assert_eq!((v.width, v.height), (240, 320), "probe reports the upright size");
    let f = thumbnail(&path, 1.0, (160, 160)).unwrap();
    assert_eq!((f.width, f.height), (120, 160));
}

#[test]
fn hd_colours_use_the_bt709_matrix() {
    // A flat colour encoded as BT.709 (what HD cameras and phones write).
    let Some(path) = make(
        "bt709_red.mp4",
        &[
            "-f", "lavfi", "-i", "color=c=0xC04040:s=1280x720:d=1",
            "-vf", "scale=out_color_matrix=bt709:out_range=tv,format=yuv420p",
            "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709",
            "-c:v", "libx264", "-qp", "0",
        ],
    ) else {
        return;
    };
    let f = thumbnail(&path, 0.5, (64, 64)).unwrap();
    let p = &f.rgba[(f.rgba.len() / 2) & !3..][..3];
    // FFmpeg's own BT.709 decode of this file is (189, 61, 61); read as BT.601 (the old
    // default) it would be (178, 48, 62).
    for (got, want) in p.iter().zip([189, 61, 61]) {
        assert!((*got as i32 - want).abs() <= 5, "got {p:?}, want about (189, 61, 61)");
    }
    assert!(p[0] > 183 && p[1] > 54, "looks like BT.601: {p:?}");
}
