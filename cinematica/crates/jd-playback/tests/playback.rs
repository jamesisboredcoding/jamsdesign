//! Mixing and playback against a test clip made with the `ffmpeg` CLI (skipped without it).
//! Playback tests run at 2x, which plays silently, so tests never make sound.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use jd_playback::{Mixer, Request, SequencePlayer};
use jd_timeline::{ClipSource, FrameRate, Sequence, from_seconds};

/// 2 s of 25 fps testsrc with a 440 Hz tone.
fn fixture() -> Option<PathBuf> {
    static FIXTURE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let dir = std::env::temp_dir().join("jd-playback-tests");
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join("av_2s.mp4");
            if path.exists() {
                return Some(path);
            }
            let tmp = dir.join(format!("tmp-{}.mp4", std::process::id()));
            let ok = Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=25"])
                .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"])
                .args(["-c:v", "libx264", "-g", "25", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
                .arg(&tmp)
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                eprintln!("skipping: couldn't make a test video with the ffmpeg CLI");
                return None;
            }
            std::fs::rename(&tmp, &path).ok()?;
            Some(path)
        })
        .clone()
}

fn clip(seconds: f64) -> ClipSource {
    ClipSource { media: 1, name: "av".into(), source_in: 0, duration: from_seconds(seconds), media_duration: Some(from_seconds(2.0)) }
}

/// The tone from 0 to 1 s on A1 and from 2 to 3 s on A2 (a gap between).
fn sequence() -> Sequence {
    let mut s = Sequence::new("s", FrameRate::new(25, 1), (160, 120), 2, 2);
    s.overwrite(0, Some(&clip(1.0)), Some(&clip(1.0))).unwrap();
    s.audio[0].targeted = false;
    s.audio[1].targeted = true;
    s.overwrite(from_seconds(2.0), None, Some(&clip(1.0))).unwrap();
    s
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|v| v * v).sum::<f32>() / buf.len().max(1) as f32).sqrt()
}

/// Mixes the whole sequence and returns the loudness of each 0.5 s.
fn loudness(seq: &Sequence, media: &HashMap<u64, PathBuf>) -> Vec<f32> {
    let mut m = Mixer::new(seq, media, 48_000, 0);
    (0..6).map(|_| rms(&m.mix(24_000))).collect()
}

#[test]
fn mixer_plays_clips_where_they_are_and_silence_in_gaps() {
    let Some(path) = fixture() else { return };
    let media = HashMap::from([(1, path)]);
    let seq = sequence();
    let l = loudness(&seq, &media);
    assert!(l[0] > 0.02 && l[1] > 0.02, "tone in the first second: {l:?}");
    assert!(l[2] < 1e-4 && l[3] < 1e-4, "silent gap: {l:?}");
    assert!(l[4] > 0.02 && l[5] > 0.02, "tone on A2: {l:?}");
}

#[test]
fn muted_tracks_are_silent_and_solo_wins() {
    let Some(path) = fixture() else { return };
    let media = HashMap::from([(1, path)]);
    let mut seq = sequence();
    seq.audio[0].enabled = false;
    let l = loudness(&seq, &media);
    assert!(l[0] < 1e-4 && l[4] > 0.02, "A1 muted: {l:?}");
    seq.audio[0].enabled = true;
    seq.audio[0].solo = true;
    let l = loudness(&seq, &media);
    assert!(l[0] > 0.02 && l[4] < 1e-4, "A1 soloed: {l:?}");
}

#[test]
fn mixing_from_the_middle_starts_inside_the_clip() {
    let Some(path) = fixture() else { return };
    let media = HashMap::from([(1, path)]);
    let mut m = Mixer::new(&sequence(), &media, 48_000, from_seconds(0.5));
    assert!(rms(&m.mix(12_000)) > 0.02);
    // 0.75..1.0 s still has tone, then the gap.
    assert!(rms(&m.mix(12_000)) > 0.02);
    assert!(rms(&m.mix(12_000)) < 1e-4);
}

#[test]
fn player_plays_to_the_end_showing_frames_then_black() {
    let Some(path) = fixture() else { return };
    let media = HashMap::from([(1, path)]);
    let seq = sequence();
    let end = seq.duration();
    let player = SequencePlayer::new();
    player.play(Request { sequence: seq, media, from: 0, speed: 2.0, max_size: (160, 160) });
    let start = Instant::now();
    let mut frames = 0;
    let mut black = 0;
    let mut last = 0;
    while player.playing() {
        assert!(start.elapsed() < Duration::from_secs(10), "playback didn't finish");
        let p = player.position();
        assert!(p >= last, "position only moves forward");
        last = p;
        match player.take_frame() {
            Some(Some(_)) => frames += 1,
            Some(None) => black += 1,
            None => {}
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(player.position(), end);
    assert!(frames >= 5, "{frames} frames shown");
    assert!(black >= 1, "black after the video ends at 1 s");
    // 3 s at 2x: about 1.5 s.
    assert!(start.elapsed() < Duration::from_secs(4));
    assert!(player.take_error().is_none());
}

#[test]
fn stop_stops_and_play_restarts_from_a_new_place() {
    let Some(path) = fixture() else { return };
    let media = HashMap::from([(1, path)]);
    let player = SequencePlayer::new();
    let req = Request { sequence: sequence(), media, from: 0, speed: 2.0, max_size: (64, 64) };
    player.play(req.clone());
    std::thread::sleep(Duration::from_millis(100));
    player.stop();
    assert!(!player.playing());
    player.play(Request { from: from_seconds(2.5), ..req });
    std::thread::sleep(Duration::from_millis(50));
    assert!(player.position() >= from_seconds(2.5));
}
