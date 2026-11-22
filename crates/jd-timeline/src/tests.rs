//! Edit operations, checked against what Premiere does.

use crate::*;

const RATE: FrameRate = FrameRate::new(25, 1);

/// `n` frames.
fn f(n: i64) -> Time {
    RATE.from_frames(n)
}

fn src(media: MediaId, name: &str, source_in: i64, frames: i64) -> ClipSource {
    ClipSource { media, name: name.into(), source_in: f(source_in), duration: f(frames), media_duration: Some(f(1000)) }
}

fn seq() -> Sequence {
    Sequence::new("Sequence 01", RATE, (1920, 1080), 3, 3)
}

/// (start, end) of each clip on a track, in frames.
fn spans(s: &Sequence, r: TrackRef) -> Vec<(i64, i64)> {
    s.track(r).clips.iter().map(|c| (RATE.to_frames(c.start), RATE.to_frames(c.end()))).collect()
}

const V1: TrackRef = TrackRef::video(0);
const V2: TrackRef = TrackRef::video(1);
const A1: TrackRef = TrackRef::audio(0);
const A2: TrackRef = TrackRef::audio(1);

/// V1/A1: [0,100) "a" linked with its audio, [100,150) "b" linked, both from media 1/2.
fn two_clips() -> (Sequence, Vec<ClipId>, Vec<ClipId>) {
    let mut s = seq();
    let a = s.overwrite(0, Some(&src(1, "a", 0, 100)), Some(&src(1, "a", 0, 100))).unwrap();
    let b = s.overwrite(f(100), Some(&src(2, "b", 10, 50)), Some(&src(2, "b", 10, 50))).unwrap();
    (s, a, b)
}

#[test]
fn overwrite_places_linked_video_and_audio_on_targets() {
    let (s, a, b) = two_clips();
    s.check();
    assert_eq!(spans(&s, V1), [(0, 100), (100, 150)]);
    assert_eq!(spans(&s, A1), [(0, 100), (100, 150)]);
    assert_eq!(s.linked(a[0]), a);
    assert_ne!(s.clip(a[0]).unwrap().link, s.clip(b[0]).unwrap().link);
    assert_eq!(s.duration(), f(150));
}

#[test]
fn overwrite_inside_a_clip_splits_it_around_the_new_one() {
    let (mut s, ..) = two_clips();
    s.overwrite(f(20), Some(&src(3, "c", 0, 10)), None).unwrap();
    s.check();
    assert_eq!(spans(&s, V1), [(0, 20), (20, 30), (30, 100), (100, 150)]);
    // The right part continues where the media left off.
    assert_eq!(s.video[0].clips[2].source_in, f(30));
    // Audio untouched (only video was given).
    assert_eq!(spans(&s, A1), [(0, 100), (100, 150)]);
}

#[test]
fn insert_pushes_every_unlocked_track_along() {
    let (mut s, ..) = two_clips();
    s.video[1].targeted = false;
    s.audio[1].locked = true;
    s.overwrite(f(0), None, None).unwrap_err();
    s.track_mut(V2).clips.push(Clip { id: 999, media: 9, name: "v2".into(), start: f(110), duration: f(10), source_in: 0, media_duration: None, link: None });
    s.insert(f(50), Some(&src(5, "ins", 0, 25)), Some(&src(5, "ins", 0, 25))).unwrap();
    s.check();
    assert_eq!(spans(&s, V1), [(0, 50), (50, 75), (75, 125), (125, 175)]);
    assert_eq!(s.video[0].clips[2].source_in, f(50), "a's right half continues its media");
    assert_eq!(spans(&s, A1), [(0, 50), (50, 75), (75, 125), (125, 175)]);
    assert_eq!(spans(&s, V2), [(135, 145)]);
}

#[test]
fn no_targets_is_an_error() {
    let mut s = seq();
    s.video[0].targeted = false;
    s.audio[0].targeted = false;
    assert_eq!(s.insert(0, Some(&src(1, "a", 0, 10)), Some(&src(1, "a", 0, 10))), Err(EditError::NoTarget));
}

#[test]
fn lift_leaves_a_gap_extract_closes_it() {
    let (mut s, ..) = two_clips();
    let mut e = s.clone();
    s.lift(f(90), f(110)).unwrap();
    s.check();
    assert_eq!(spans(&s, V1), [(0, 90), (110, 150)]);
    e.extract(f(90), f(110));
    e.check();
    assert_eq!(spans(&e, V1), [(0, 90), (90, 130)]);
    assert_eq!(spans(&e, A1), [(0, 90), (90, 130)]);
    assert_eq!(e.video[0].clips[1].source_in, f(20), "b now starts 10 frames later in its media");
}

#[test]
fn razor_splits_and_keeps_halves_linked_to_each_other() {
    let (mut s, a, _) = two_clips();
    assert_eq!(s.razor(f(40), &[V1, A1, V2]), 2);
    s.check();
    assert_eq!(spans(&s, V1), [(0, 40), (40, 100), (100, 150)]);
    let right_v = s.video[0].clips[1].id;
    let right_a = s.audio[0].clips[1].id;
    let mut linked = s.linked(right_v);
    linked.sort();
    assert_eq!(linked, [right_v.min(right_a), right_v.max(right_a)]);
    assert_eq!(s.linked(a[0]), a, "left halves keep the original link");
    // Cutting exactly on an edit point does nothing.
    assert_eq!(s.razor(f(100), &[V1]), 0);
}

#[test]
fn ripple_delete_closes_the_gap_on_all_unlocked_tracks() {
    let (mut s, a, _) = two_clips();
    s.ripple_delete(&a).unwrap();
    s.check();
    assert_eq!(spans(&s, V1), [(0, 50)]);
    assert_eq!(spans(&s, A1), [(0, 50)]);
}

#[test]
fn ripple_delete_is_blocked_by_clips_on_other_tracks() {
    let (mut s, a, _) = two_clips();
    s.track_mut(V2).clips.push(Clip { id: 999, media: 9, name: "t".into(), start: f(10), duration: f(10), source_in: 0, media_duration: None, link: None });
    let before = s.clone();
    // Deleting only the video leaves its audio in the gap.
    assert_eq!(s.ripple_delete(&a[..1]), Err(EditError::Blocked));
    assert_eq!(s, before, "nothing changes when blocked");
    // Locking the blocking tracks lets it ripple; they stay put.
    s.video[1].locked = true;
    s.audio[0].locked = true;
    s.ripple_delete(&a[..1]).unwrap();
    assert_eq!(spans(&s, V1), [(0, 50)]);
    assert_eq!(spans(&s, A1), [(0, 100), (100, 150)]);
    assert_eq!(spans(&s, V2), [(10, 20)]);
}

#[test]
fn move_overwrites_at_the_destination_and_clamps_at_zero() {
    let (mut s, a, b) = two_clips();
    // Move b (with its audio) up to V2 and back 30 frames.
    let mut ids = s.linked(b[0]);
    ids.sort();
    assert_eq!(s.move_clips(&ids, -f(30), 1, 0).unwrap(), -f(30));
    s.check();
    assert_eq!(spans(&s, V2), [(70, 120)]);
    assert_eq!(spans(&s, V1), [(0, 100)]);
    assert_eq!(spans(&s, A1), [(0, 70), (70, 120)], "audio overwrote the end of a's audio");
    // Can't go before 0.
    assert_eq!(s.move_clips(&a, -f(500), 0, 0).unwrap(), 0);
    // No track above V3.
    assert_eq!(s.move_clips(&ids, 0, 5, 0), Err(EditError::Blocked));
    // Audio moves to A2.
    s.move_clips(&ids, 0, 0, 1).unwrap();
    assert_eq!(spans(&s, A2), [(70, 120)]);
}

#[test]
fn trim_is_limited_by_neighbours_media_and_one_frame() {
    let (mut s, a, b) = two_clips();
    // b starts at 10 in its media: its In edge can go back 10 frames, but a is in the way.
    assert_eq!(s.trim(&b, Edge::In, -f(5)), 0);
    s.lift(f(90), f(100)).unwrap();
    assert_eq!(s.trim(&b, Edge::In, -f(50)), -f(10), "limited by the media start and a's end");
    s.check();
    assert_eq!(spans(&s, V1), [(0, 90), (90, 150)]);
    // Out edge: at least one frame stays.
    assert_eq!(s.trim(&a, Edge::Out, -f(500)), -f(89));
    // Out edge can't pass the media's end (1000 frames; b uses 0..60).
    assert_eq!(s.trim(&b, Edge::Out, f(5000)), f(940));
    s.check();
}

#[test]
fn ripple_trim_moves_everything_after() {
    let (mut s, a, _) = two_clips();
    s.track_mut(V2).clips.push(Clip { id: 999, media: 9, name: "t".into(), start: f(120), duration: f(10), source_in: 0, media_duration: None, link: None });
    // Shorten a by 20: b and the V2 title follow.
    assert_eq!(s.ripple_trim(&a, Edge::Out, -f(20)), -f(20));
    s.check();
    assert_eq!(spans(&s, V1), [(0, 80), (80, 130)]);
    assert_eq!(spans(&s, A1), [(0, 80), (80, 130)]);
    assert_eq!(spans(&s, V2), [(100, 110)]);
    // Trim a's head by 10: a keeps its start, shows later frames, everything after moves back.
    assert_eq!(s.ripple_trim(&a, Edge::In, f(10)), f(10));
    assert_eq!(spans(&s, V1), [(0, 70), (70, 120)]);
    assert_eq!(s.video[0].clips[0].source_in, f(10));
    // Shortening can't pull clips on another track over earlier content.
    let mut t = seq();
    let x = t.overwrite(0, Some(&src(1, "x", 0, 100)), None).unwrap();
    t.track_mut(V2).clips.push(Clip { id: 999, media: 9, name: "t".into(), start: f(90), duration: f(30), source_in: 0, media_duration: None, link: None });
    assert_eq!(t.ripple_trim(&x, Edge::Out, -f(20)), 0, "a V2 clip spans the edit point");
}

#[test]
fn slip_changes_media_frames_not_position() {
    let (mut s, _, b) = two_clips();
    assert_eq!(s.slip(&b, -f(50)), -f(10), "b's media starts at 10");
    assert_eq!(spans(&s, V1), [(0, 100), (100, 150)]);
    assert_eq!(s.video[0].clips[1].source_in, 0);
    assert_eq!(s.slip(&b, f(5000)), f(950));
    s.check();
}

#[test]
fn snapping_finds_the_nearest_edge_or_extra_point() {
    let (s, a, _) = two_clips();
    assert_eq!(s.snap(f(98), f(3), &[], &[]), Some(f(100)));
    assert_eq!(s.snap(f(60), f(3), &[], &[]), None);
    assert_eq!(s.snap(f(60), f(3), &[], &[f(62)]), Some(f(62)));
    // Ignoring a's edges (it's the one being dragged) still sees b's start at 100.
    assert_eq!(s.snap(f(99), f(3), &a, &[]), Some(f(100)));
    assert_eq!(s.edit_points(), [0, f(100), f(150)]);
}

#[test]
fn top_video_wins_and_hidden_tracks_are_skipped() {
    let (mut s, ..) = two_clips();
    s.track_mut(V2).clips.push(Clip { id: 999, media: 9, name: "t".into(), start: f(10), duration: f(10), source_in: 0, media_duration: None, link: None });
    assert_eq!(s.top_video_at(f(15)).unwrap().name, "t");
    s.video[1].enabled = false;
    assert_eq!(s.top_video_at(f(15)).unwrap().name, "a");
    assert!(s.top_video_at(f(200)).is_none());
}
