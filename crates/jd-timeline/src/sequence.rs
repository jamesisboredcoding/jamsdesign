//! Sequences, tracks and clips, and every edit as a function on [`Sequence`].
//!
//! Tracks keep their clips sorted and never overlapping. "Unlocked" tracks act like
//! Premiere's sync lock: inserts, extracts and ripples move everything on every unlocked
//! track together, so picture and sound stay in sync. Lock a track to keep it still.

use std::collections::HashMap;

use crate::time::{FrameRate, Time};

pub type MediaId = u64;
pub type ClipId = u64;
pub type LinkId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TrackKind {
    Video,
    Audio,
}

/// A track by kind and index (`V1` is `Video, 0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrackRef {
    pub kind: TrackKind,
    pub index: usize,
}

impl TrackRef {
    pub const fn video(index: usize) -> Self {
        Self { kind: TrackKind::Video, index }
    }

    pub const fn audio(index: usize) -> Self {
        Self { kind: TrackKind::Audio, index }
    }

    pub fn name(self) -> String {
        let p = if self.kind == TrackKind::Video { "V" } else { "A" };
        format!("{p}{}", self.index + 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub id: ClipId,
    pub media: MediaId,
    pub name: String,
    /// Position in the sequence.
    pub start: Time,
    pub duration: Time,
    /// Where in the media the clip starts.
    pub source_in: Time,
    /// The media's length, limiting trims (`None`: unlimited, e.g. a still image).
    pub media_duration: Option<Time>,
    /// Clips sharing a link move, trim and select together (a video clip and its audio).
    pub link: Option<LinkId>,
}

impl Clip {
    pub fn end(&self) -> Time {
        self.start + self.duration
    }

    pub fn source_out(&self) -> Time {
        self.source_in + self.duration
    }

    pub fn contains(&self, t: Time) -> bool {
        self.start <= t && t < self.end()
    }

    pub fn overlaps(&self, a: Time, b: Time) -> bool {
        self.start < b && a < self.end()
    }

    /// Media time shown at sequence time `t`.
    pub fn source_time(&self, t: Time) -> Time {
        self.source_in + (t - self.start)
    }
}

/// Media to put into a sequence (a Source monitor In/Out range).
#[derive(Debug, Clone, PartialEq)]
pub struct ClipSource {
    pub media: MediaId,
    pub name: String,
    pub source_in: Time,
    pub duration: Time,
    pub media_duration: Option<Time>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub clips: Vec<Clip>,
    pub locked: bool,
    /// Video: shown. Audio: not muted.
    pub enabled: bool,
    pub solo: bool,
    /// Insert/overwrite/lift land on targeted tracks.
    pub targeted: bool,
}

impl Track {
    fn new(targeted: bool) -> Self {
        Self { clips: Vec::new(), locked: false, enabled: true, solo: false, targeted }
    }

    fn sort(&mut self) {
        self.clips.sort_by_key(|c| c.start);
    }

    fn place(&mut self, clip: Clip) {
        debug_assert!(self.clips.iter().all(|c| !c.overlaps(clip.start, clip.end())), "overlapping clip");
        self.clips.push(clip);
        self.sort();
    }

    /// No clip (other than `ignore`) in `[a, b)`.
    pub fn is_free(&self, a: Time, b: Time, ignore: &[ClipId]) -> bool {
        self.clips.iter().all(|c| ignore.contains(&c.id) || !c.overlaps(a, b))
    }

    pub fn clip_at(&self, t: Time) -> Option<&Clip> {
        self.clips.iter().find(|c| c.contains(t))
    }
}

/// Gives split-off halves new ids, and keeps halves of linked clips linked to each other.
struct Ids<'a> {
    next: &'a mut u64,
    relink: HashMap<LinkId, LinkId>,
}

impl Ids<'_> {
    fn id(&mut self) -> u64 {
        let id = *self.next;
        *self.next += 1;
        id
    }

    fn link_for(&mut self, old: Option<LinkId>) -> Option<LinkId> {
        let old = old?;
        if let Some(&l) = self.relink.get(&old) {
            return Some(l);
        }
        let l = self.id();
        self.relink.insert(old, l);
        Some(l)
    }
}

/// Splits the clip spanning `at` (if any); returns the new right half's id.
fn split(track: &mut Track, at: Time, ids: &mut Ids) -> Option<ClipId> {
    let i = track.clips.iter().position(|c| c.start < at && at < c.end())?;
    let left = &mut track.clips[i];
    let mut right = left.clone();
    let cut = at - left.start;
    left.duration = cut;
    right.id = ids.id();
    right.link = ids.link_for(right.link);
    right.start = at;
    right.source_in += cut;
    right.duration -= cut;
    let id = right.id;
    track.clips.insert(i + 1, right);
    Some(id)
}

/// Empties `[a, b)`: clips inside go, clips across the edges are trimmed back.
fn clear(track: &mut Track, a: Time, b: Time, ids: &mut Ids) {
    if b <= a {
        return;
    }
    split(track, a, ids);
    split(track, b, ids);
    track.clips.retain(|c| !c.overlaps(a, b));
}

/// Moves every clip starting at or after `at` by `delta`.
fn shift_from(track: &mut Track, at: Time, delta: Time) {
    for c in track.clips.iter_mut().filter(|c| c.start >= at) {
        c.start += delta;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// No targeted, unlocked track to edit into.
    NoTarget,
    /// The clips are on a locked track.
    Locked,
    /// Other tracks have clips in the way (lock them to ripple anyway).
    Blocked,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EditError::NoTarget => "no targeted track (click a track's V1/A1 button to target it)",
            EditError::Locked => "the track is locked",
            EditError::Blocked => "clips on other tracks are in the way (lock those tracks to ripple anyway)",
        })
    }
}

impl std::error::Error for EditError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub name: String,
    pub rate: FrameRate,
    pub width: u32,
    pub height: u32,
    pub sample_rate: u32,
    /// V1 first (drawn at the bottom of the video tracks).
    pub video: Vec<Track>,
    /// A1 first (drawn at the top of the audio tracks).
    pub audio: Vec<Track>,
    next_id: u64,
}

impl Sequence {
    /// A sequence with V1 and A1 targeted.
    pub fn new(name: impl Into<String>, rate: FrameRate, size: (u32, u32), video_tracks: usize, audio_tracks: usize) -> Self {
        Self {
            name: name.into(),
            rate,
            width: size.0,
            height: size.1,
            sample_rate: 48_000,
            video: (0..video_tracks).map(|i| Track::new(i == 0)).collect(),
            audio: (0..audio_tracks).map(|i| Track::new(i == 0)).collect(),
            next_id: 1,
        }
    }

    pub fn track(&self, r: TrackRef) -> &Track {
        match r.kind {
            TrackKind::Video => &self.video[r.index],
            TrackKind::Audio => &self.audio[r.index],
        }
    }

    pub fn track_mut(&mut self, r: TrackRef) -> &mut Track {
        match r.kind {
            TrackKind::Video => &mut self.video[r.index],
            TrackKind::Audio => &mut self.audio[r.index],
        }
    }

    pub fn track_refs(&self) -> Vec<TrackRef> {
        (0..self.video.len()).map(TrackRef::video).chain((0..self.audio.len()).map(TrackRef::audio)).collect()
    }

    fn unlocked(&self) -> Vec<TrackRef> {
        self.track_refs().into_iter().filter(|&r| !self.track(r).locked).collect()
    }

    pub fn find(&self, id: ClipId) -> Option<(TrackRef, &Clip)> {
        self.track_refs()
            .into_iter()
            .find_map(|r| self.track(r).clips.iter().find(|c| c.id == id).map(|c| (r, c)))
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.find(id).map(|(_, c)| c)
    }

    /// End of the last clip.
    pub fn duration(&self) -> Time {
        self.video.iter().chain(&self.audio).flat_map(|t| t.clips.last()).map(Clip::end).max().unwrap_or(0)
    }

    /// `id` and every clip linked to it.
    pub fn linked(&self, id: ClipId) -> Vec<ClipId> {
        let Some(link) = self.clip(id).and_then(|c| c.link) else { return vec![id] };
        self.video.iter().chain(&self.audio).flat_map(|t| &t.clips).filter(|c| c.link == Some(link)).map(|c| c.id).collect()
    }

    /// First targeted, unlocked video and audio tracks.
    pub fn targets(&self) -> (Option<TrackRef>, Option<TrackRef>) {
        let pick = |kind| self.track_refs().into_iter().find(|&r| r.kind == kind && self.track(r).targeted && !self.track(r).locked);
        (pick(TrackKind::Video), pick(TrackKind::Audio))
    }

    /// Every clip start and end, sorted, without duplicates (Up/Down and snapping).
    pub fn edit_points(&self) -> Vec<Time> {
        let mut pts: Vec<Time> = self.video.iter().chain(&self.audio).flat_map(|t| &t.clips).flat_map(|c| [c.start, c.end()]).collect();
        pts.sort_unstable();
        pts.dedup();
        pts
    }

    /// The clip the Program monitor shows at `t`: the topmost enabled video track wins.
    pub fn top_video_at(&self, t: Time) -> Option<&Clip> {
        self.video.iter().rev().filter(|tr| tr.enabled).find_map(|tr| tr.clip_at(t))
    }

    fn make_clip(&mut self, s: &ClipSource, start: Time, link: Option<LinkId>) -> Clip {
        let id = self.next_id;
        self.next_id += 1;
        Clip {
            id,
            media: s.media,
            name: s.name.clone(),
            start,
            duration: s.duration,
            source_in: s.source_in,
            media_duration: s.media_duration,
            link,
        }
    }

    /// The targeted tracks for each part, and a link id when both get placed.
    fn plan(&mut self, video: Option<&ClipSource>, audio: Option<&ClipSource>) -> Result<Vec<(TrackRef, ClipSource)>, EditError> {
        let (tv, ta) = self.targets();
        let mut parts = Vec::new();
        if let (Some(v), Some(t)) = (video, tv) {
            parts.push((t, v.clone()));
        }
        if let (Some(a), Some(t)) = (audio, ta) {
            parts.push((t, a.clone()));
        }
        if parts.is_empty() { Err(EditError::NoTarget) } else { Ok(parts) }
    }

    fn place_parts(&mut self, at: Time, parts: Vec<(TrackRef, ClipSource)>) -> Vec<ClipId> {
        let link = (parts.len() > 1).then(|| {
            let l = self.next_id;
            self.next_id += 1;
            l
        });
        parts
            .into_iter()
            .map(|(r, s)| {
                let clip = self.make_clip(&s, at, link);
                let id = clip.id;
                self.track_mut(r).place(clip);
                id
            })
            .collect()
    }

    /// Insert (`,`): opens a gap at `at` on every unlocked track and puts the source on the
    /// targeted tracks.
    pub fn insert(&mut self, at: Time, video: Option<&ClipSource>, audio: Option<&ClipSource>) -> Result<Vec<ClipId>, EditError> {
        let parts = self.plan(video, audio)?;
        let len = parts.iter().map(|(_, s)| s.duration).max().unwrap_or(0);
        let next = &mut self.next_id;
        let mut ids = Ids { next, relink: HashMap::new() };
        for t in self.video.iter_mut().chain(self.audio.iter_mut()).filter(|t| !t.locked) {
            split(t, at, &mut ids);
            shift_from(t, at, len);
        }
        Ok(self.place_parts(at, parts))
    }

    /// Overwrite (`.`): replaces whatever is on the targeted tracks from `at`.
    pub fn overwrite(&mut self, at: Time, video: Option<&ClipSource>, audio: Option<&ClipSource>) -> Result<Vec<ClipId>, EditError> {
        let parts = self.plan(video, audio)?;
        for (r, s) in &parts {
            let (r, d) = (*r, s.duration);
            let next = &mut self.next_id;
            let track = match r.kind {
                TrackKind::Video => &mut self.video[r.index],
                TrackKind::Audio => &mut self.audio[r.index],
            };
            clear(track, at, at + d, &mut Ids { next, relink: HashMap::new() });
        }
        Ok(self.place_parts(at, parts))
    }

    /// Lift (`;`): removes `[a, b)` from the targeted tracks, leaving a gap.
    pub fn lift(&mut self, a: Time, b: Time) -> Result<(), EditError> {
        let targets: Vec<TrackRef> = self.unlocked().into_iter().filter(|&r| self.track(r).targeted).collect();
        if targets.is_empty() {
            return Err(EditError::NoTarget);
        }
        let mut ids = Ids { next: &mut self.next_id, relink: HashMap::new() };
        for r in targets {
            let t = match r.kind {
                TrackKind::Video => &mut self.video[r.index],
                TrackKind::Audio => &mut self.audio[r.index],
            };
            clear(t, a, b, &mut ids);
        }
        Ok(())
    }

    /// Extract (`'`): removes `[a, b)` from every unlocked track and closes the gap.
    pub fn extract(&mut self, a: Time, b: Time) {
        if b <= a {
            return;
        }
        let mut ids = Ids { next: &mut self.next_id, relink: HashMap::new() };
        for t in self.video.iter_mut().chain(self.audio.iter_mut()).filter(|t| !t.locked) {
            clear(t, a, b, &mut ids);
            shift_from(t, b, a - b);
        }
    }

    /// Razor / Add Edit: cuts the clips under `at` on `tracks` (locked ones are skipped).
    /// Returns how many clips were cut.
    pub fn razor(&mut self, at: Time, tracks: &[TrackRef]) -> usize {
        let tracks: Vec<TrackRef> = tracks.iter().copied().filter(|&r| !self.track(r).locked).collect();
        let mut ids = Ids { next: &mut self.next_id, relink: HashMap::new() };
        let mut cut = 0;
        for r in tracks {
            let t = match r.kind {
                TrackKind::Video => &mut self.video[r.index],
                TrackKind::Audio => &mut self.audio[r.index],
            };
            cut += usize::from(split(t, at, &mut ids).is_some());
        }
        cut
    }

    /// Removes clips, leaving gaps. Clips on locked tracks stay.
    pub fn delete(&mut self, ids: &[ClipId]) -> usize {
        let mut n = 0;
        for t in self.video.iter_mut().chain(self.audio.iter_mut()).filter(|t| !t.locked) {
            let before = t.clips.len();
            t.clips.retain(|c| !ids.contains(&c.id));
            n += before - t.clips.len();
        }
        n
    }

    /// Removes clips and closes their gaps on every unlocked track. Fails, changing
    /// nothing, if another unlocked track has clips inside a gap.
    pub fn ripple_delete(&mut self, ids: &[ClipId]) -> Result<(), EditError> {
        let mut ranges: Vec<(Time, Time)> = ids.iter().filter_map(|&id| self.find(id)).filter(|(r, _)| !self.track(*r).locked).map(|(_, c)| (c.start, c.end())).collect();
        if ranges.is_empty() {
            return Err(EditError::Locked);
        }
        let mut next = self.clone();
        next.delete(ids);
        // Merge touching/overlapping ranges, then close them from the last one back.
        ranges.sort_unstable();
        let mut merged: Vec<(Time, Time)> = Vec::new();
        for (a, b) in ranges {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        for &(a, b) in merged.iter().rev() {
            for r in next.unlocked() {
                if !next.track(r).is_free(a, b, &[]) {
                    return Err(EditError::Blocked);
                }
            }
            for r in next.unlocked() {
                shift_from(next.track_mut(r), b, a - b);
            }
        }
        *self = next;
        Ok(())
    }

    /// Moves clips by `delta` in time and by whole tracks (video and audio separately; up is
    /// +1 for video, down is +1 for audio). Clips land as an overwrite. Returns the time
    /// delta actually applied (clamped so nothing goes before 0).
    pub fn move_clips(&mut self, ids: &[ClipId], delta: Time, video_tracks: isize, audio_tracks: isize) -> Result<Time, EditError> {
        let moving: Vec<(TrackRef, Clip)> = ids.iter().filter_map(|&id| self.find(id)).map(|(r, c)| (r, c.clone())).collect();
        if moving.is_empty() {
            return Ok(0);
        }
        if moving.iter().any(|(r, _)| self.track(*r).locked) {
            return Err(EditError::Locked);
        }
        let earliest = moving.iter().map(|(_, c)| c.start).min().unwrap_or(0);
        let delta = delta.max(-earliest);
        let dest = |r: TrackRef| -> Option<TrackRef> {
            let (d, len) = match r.kind {
                TrackKind::Video => (video_tracks, self.video.len()),
                TrackKind::Audio => (audio_tracks, self.audio.len()),
            };
            let i = r.index as isize + d;
            (0..len as isize).contains(&i).then(|| TrackRef { kind: r.kind, index: i as usize })
        };
        let mut placed = Vec::new();
        for (r, c) in &moving {
            let to = dest(*r).ok_or(EditError::Blocked)?;
            if self.track(to).locked {
                return Err(EditError::Locked);
            }
            placed.push((to, c.clone()));
        }
        self.delete(ids);
        for (to, mut c) in placed {
            c.start += delta;
            let next = &mut self.next_id;
            let t = match to.kind {
                TrackKind::Video => &mut self.video[to.index],
                TrackKind::Audio => &mut self.audio[to.index],
            };
            clear(t, c.start, c.end(), &mut Ids { next, relink: HashMap::new() });
            t.place(c);
        }
        Ok(delta)
    }

    /// How far each clip's `edge` may move: (most negative, most positive).
    fn trim_limits(&self, r: TrackRef, c: &Clip, edge: Edge, ignore: &[ClipId]) -> (Time, Time) {
        let frame = self.rate.frame();
        let others = self.track(r).clips.iter().filter(|o| !ignore.contains(&o.id));
        match edge {
            Edge::In => {
                let prev_end = others.filter(|o| o.end() <= c.start).map(Clip::end).max().unwrap_or(0);
                (-(c.source_in.min(c.start - prev_end)), c.duration - frame)
            }
            Edge::Out => {
                let next_start = others.filter(|o| o.start >= c.end()).map(|o| o.start).min().unwrap_or(Time::MAX);
                let media_left = c.media_duration.map_or(Time::MAX, |d| d - c.source_out());
                (-(c.duration - frame), media_left.min(next_start.saturating_sub(c.end())))
            }
        }
    }

    /// Trims the `edge` of clips by `delta` (one value for all, so linked clips stay
    /// together), clamped by the media's length, neighbours and a one-frame minimum.
    /// Returns the delta applied.
    pub fn trim(&mut self, ids: &[ClipId], edge: Edge, delta: Time) -> Time {
        let clips: Vec<(TrackRef, Clip)> = ids.iter().filter_map(|&id| self.find(id)).filter(|(r, _)| !self.track(*r).locked).map(|(r, c)| (r, c.clone())).collect();
        let (mut lo, mut hi) = (Time::MIN, Time::MAX);
        for (r, c) in &clips {
            let (l, h) = self.trim_limits(*r, c, edge, ids);
            lo = lo.max(l);
            hi = hi.min(h);
        }
        if clips.is_empty() || lo > hi {
            return 0;
        }
        let d = delta.clamp(lo, hi);
        for (r, c) in clips {
            if let Some(clip) = self.track_mut(r).clips.iter_mut().find(|x| x.id == c.id) {
                match edge {
                    Edge::In => {
                        clip.start += d;
                        clip.source_in += d;
                        clip.duration -= d;
                    }
                    Edge::Out => clip.duration += d,
                }
            }
        }
        d
    }

    /// Ripple trim (B): trims the edge and moves everything after the clips on every
    /// unlocked track to follow, so no gap opens or closes elsewhere. Clips keep their
    /// start; trimming the In edge changes which frame they start on. Returns the delta.
    pub fn ripple_trim(&mut self, ids: &[ClipId], edge: Edge, delta: Time) -> Time {
        let clips: Vec<(TrackRef, Clip)> = ids.iter().filter_map(|&id| self.find(id)).filter(|(r, _)| !self.track(*r).locked).map(|(r, c)| (r, c.clone())).collect();
        let Some(pivot) = clips.iter().map(|(_, c)| c.end()).max() else { return 0 };
        let frame = self.rate.frame();
        let (mut lo, mut hi) = (Time::MIN, Time::MAX);
        for (_, c) in &clips {
            let media_left = c.media_duration.map_or(Time::MAX, |d| d - c.source_out());
            let (l, h) = match edge {
                Edge::Out => (-(c.duration - frame), media_left),
                Edge::In => (-c.source_in, c.duration - frame),
            };
            lo = lo.max(l);
            hi = hi.min(h);
        }
        // Shortening pulls later content back: the space before the pivot must be empty
        // on every other unlocked track.
        let room = self
            .unlocked()
            .into_iter()
            .map(|r| {
                let t = self.track(r);
                if t.clips.iter().any(|o| !ids.contains(&o.id) && o.start < pivot && o.end() > pivot) {
                    return 0;
                }
                pivot - t.clips.iter().filter(|o| !ids.contains(&o.id) && o.end() <= pivot).map(Clip::end).max().unwrap_or(0)
            })
            .min()
            .unwrap_or(Time::MAX);
        match edge {
            Edge::Out => lo = lo.max(-room),
            Edge::In => hi = hi.min(room),
        }
        if lo > hi {
            return 0;
        }
        let d = delta.clamp(lo, hi);
        let shift = if edge == Edge::Out { d } else { -d };
        for r in self.unlocked() {
            shift_from(self.track_mut(r), pivot, shift);
        }
        for (r, c) in clips {
            if let Some(clip) = self.track_mut(r).clips.iter_mut().find(|x| x.id == c.id) {
                match edge {
                    Edge::Out => clip.duration += d,
                    Edge::In => {
                        clip.source_in += d;
                        clip.duration -= d;
                    }
                }
            }
        }
        d
    }

    /// Slip (Y): shows a different part of the media in the same slot. Returns the delta.
    pub fn slip(&mut self, ids: &[ClipId], delta: Time) -> Time {
        let clips: Vec<(TrackRef, Clip)> = ids.iter().filter_map(|&id| self.find(id)).filter(|(r, _)| !self.track(*r).locked).map(|(r, c)| (r, c.clone())).collect();
        let (mut lo, mut hi) = (Time::MIN, Time::MAX);
        for (_, c) in &clips {
            lo = lo.max(-c.source_in);
            hi = hi.min(c.media_duration.map_or(Time::MAX, |d| d - c.source_out()));
        }
        if clips.is_empty() || lo > hi {
            return 0;
        }
        let d = delta.clamp(lo, hi);
        for (r, c) in clips {
            if let Some(clip) = self.track_mut(r).clips.iter_mut().find(|x| x.id == c.id) {
                clip.source_in += d;
            }
        }
        d
    }

    /// Nearest snap point to `t` within `threshold` (clip edges and `extra` points such as
    /// the playhead), ignoring the edges of `ignore`.
    pub fn snap(&self, t: Time, threshold: Time, ignore: &[ClipId], extra: &[Time]) -> Option<Time> {
        let edges = self
            .video
            .iter()
            .chain(&self.audio)
            .flat_map(|tr| &tr.clips)
            .filter(|c| !ignore.contains(&c.id))
            .flat_map(|c| [c.start, c.end()]);
        edges
            .chain(extra.iter().copied())
            .chain(std::iter::once(0))
            .filter(|p| (p - t).abs() <= threshold)
            .min_by_key(|p| (p - t).abs())
    }

    #[cfg(test)]
    pub(crate) fn check(&self) {
        for t in self.video.iter().chain(&self.audio) {
            for w in t.clips.windows(2) {
                assert!(w[0].end() <= w[1].start, "overlap: {:?}", w);
            }
            for c in &t.clips {
                assert!(c.duration > 0 && c.start >= 0 && c.source_in >= 0, "bad clip {c:?}");
                if let Some(d) = c.media_duration {
                    assert!(c.source_out() <= d, "past media end {c:?}");
                }
            }
        }
    }
}
