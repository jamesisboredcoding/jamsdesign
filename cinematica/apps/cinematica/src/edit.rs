//! Timeline messages: selection, edits (each one undo step), zoom and scrolling.

use jd_timeline::{ClipId, ClipSource, Sequence, Time, TrackRef, from_seconds, to_seconds};

use crate::app::{Editor, Focus, Message, TrackToggle};
use crate::media::Kind;

/// Zoom range, in pixels per second.
pub const MIN_ZOOM: f32 = 0.5;
pub const MAX_ZOOM: f32 = 2000.0;

impl Editor {
    /// Runs an edit as one undo step. Errors restore the sequence and show the reason.
    pub fn edit(&mut self, label: &str, f: impl FnOnce(&mut Sequence) -> Result<(), String>) -> bool {
        // Editing stops playback, like Premiere.
        self.stop_playback();
        let before = self.sequence.clone();
        match f(&mut self.sequence) {
            Ok(()) => {
                if self.sequence != before {
                    self.history.push(label, before);
                    self.notice = None;
                }
                self.after_change();
                true
            }
            Err(e) => {
                self.sequence = before;
                self.notice = Some(format!("{label}: {e}"));
                false
            }
        }
    }

    /// Drops selected clips that no longer exist and refreshes the Program monitor.
    fn after_change(&mut self) {
        let seq = &self.sequence;
        self.selection.retain(|&id| seq.clip(id).is_some());
        self.sync_program();
    }

    pub fn set_playhead(&mut self, t: Time) {
        let t = self.sequence.rate.snap(t.max(0));
        self.playhead = t;
        self.focus = Focus::Timeline;
        self.keep_in_view(t);
        if self.program.is_playing() {
            // Clicking the ruler while playing carries on playing from there.
            let speed = self.program.speed;
            self.play_sequence(speed);
        } else {
            self.sync_program();
        }
    }

    /// Plays the sequence from the playhead (from the start if it's at the end).
    pub fn play_sequence(&mut self, speed: f64) {
        let end = self.sequence.duration();
        if end == 0 {
            return;
        }
        self.focus = Focus::Timeline;
        let from = if speed > 0.0 && self.playhead >= end { 0 } else { self.playhead };
        let media = self.media.iter().map(|m| (m.id, m.path.clone())).collect();
        // Tests must stay quiet: they play silently at 2x.
        let speed = if cfg!(test) && speed == 1.0 { 2.0 } else { speed };
        self.program.play(jd_playback::Request {
            sequence: self.sequence.clone(),
            media,
            from,
            speed,
            max_size: (1920, 1080),
        });
    }

    pub fn stop_playback(&mut self) {
        if self.program.is_playing() {
            let at = self.program.stop();
            self.playhead = self.sequence.rate.snap(at);
            self.sync_program();
        }
    }

    /// While playing: page the timeline along so the playhead stays in view.
    pub fn follow_playhead(&mut self) {
        self.keep_in_view(self.playhead);
    }

    /// Meters show the newest peak, or fall back by 24 dB a second (about 0.4 dB a frame).
    pub fn update_meters(&mut self) {
        let a = self.program.take_peaks();
        let b = self.source.take_peaks();
        for i in 0..2 {
            let peak = a[i].max(b[i]);
            let db = if peak > 0.0 { 20.0 * peak.log10() } else { f32::NEG_INFINITY };
            let fallen = self.meters[i] - 0.4;
            self.meters[i] = if db >= fallen { db } else if fallen < -60.0 { f32::NEG_INFINITY } else { fallen };
        }
    }

    /// Scrolls the timeline (a page at a time) so `t` is visible.
    fn keep_in_view(&mut self, t: Time) {
        let visible = from_seconds((self.timeline_width / self.px_per_second) as f64);
        if t < self.scroll {
            self.scroll = (t - visible / 10).max(0);
        } else if t > self.scroll + visible {
            self.scroll = t - visible / 10;
        }
    }

    /// Shows the frame under the playhead in the Program monitor.
    pub fn sync_program(&mut self) {
        let target = self.sequence.top_video_at(self.playhead).and_then(|c| {
            let item = self.media_item(c.media)?;
            let t = match item.kind() {
                Kind::Still => 0.0,
                _ => to_seconds(c.source_time(self.playhead)),
            };
            Some((c.media, item.path.clone(), t))
        });
        self.program.show(target);
    }

    /// What `,` and `.` put in: the Source monitor's In/Out range, or (with nothing in the
    /// Source monitor) the whole clip selected in the bin.
    fn source_clips(&self) -> Option<(Option<ClipSource>, Option<ClipSource>)> {
        let rate = self.sequence.rate;
        let (item, source_in, length) = match self.source_item() {
            Some(item) if self.source.loaded() => {
                (item, self.source.in_point.unwrap_or(0.0), self.source.marked_duration())
            }
            _ => {
                let item = self.selected_media.and_then(|id| self.media_item(id))?;
                (item, 0.0, item.duration())
            }
        };
        let info = item.info.as_ref()?;
        let still = item.kind() == Kind::Still;
        let duration = rate.snap(from_seconds(length)).max(rate.frame());
        let part = ClipSource {
            media: item.id,
            name: item.name.clone(),
            source_in: from_seconds(source_in),
            duration,
            media_duration: (!still).then(|| from_seconds(item.duration())),
        };
        let video = info.video.is_some().then(|| part.clone());
        let audio = info.audio.is_some().then_some(part);
        Some((video, audio))
    }

    /// Selection plus linked clips (with Linked Selection on).
    pub fn with_links(&self, ids: &[ClipId]) -> Vec<ClipId> {
        let mut out: Vec<ClipId> = Vec::new();
        for &id in ids {
            let group = if self.linked { self.sequence.linked(id) } else { vec![id] };
            for g in group {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
        }
        out
    }

    /// Sequence In/Out, or the clip(s) under the playhead on targeted tracks.
    fn marked_range(&self) -> Option<(Time, Time)> {
        match (self.seq_in, self.seq_out) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            (Some(a), None) => Some((a, self.sequence.duration())).filter(|(a, b)| b > a),
            (None, Some(b)) => Some((0, b)),
            _ => None,
        }
    }

    pub(crate) fn timeline_message(&mut self, message: Message) {
        // Edits at the playhead use where playback stopped (snapped to a frame).
        if matches!(message, Message::AddEdit { .. } | Message::Insert | Message::Overwrite | Message::Lift | Message::Extract) {
            self.stop_playback();
        }
        match message {
            Message::TimelineWidth(w) => self.timeline_width = w,
            Message::SetPlayhead(t) => self.set_playhead(t),
            Message::Select { id, add } => {
                self.focus = Focus::Timeline;
                let group = self.with_links(&[id]);
                if add {
                    // Shift-click toggles.
                    if self.selection.contains(&id) {
                        self.selection.retain(|s| !group.contains(s));
                    } else {
                        self.selection.extend(group);
                    }
                } else if !self.selection.contains(&id) {
                    self.selection = group;
                }
            }
            Message::Deselect => self.selection.clear(),
            Message::SelectAll => {
                self.selection = self.sequence.video.iter().chain(&self.sequence.audio).flat_map(|t| &t.clips).map(|c| c.id).collect();
            }
            Message::SelectForward { at, track } => {
                let refs = match track {
                    Some(r) => vec![r],
                    None => self.sequence.track_refs(),
                };
                let ids: Vec<ClipId> = refs.iter().flat_map(|&r| &self.sequence.track(r).clips).filter(|c| c.end() > at).map(|c| c.id).collect();
                self.selection = self.with_links(&ids);
            }
            Message::MoveClips { ids, delta, video_tracks, audio_tracks } => {
                self.edit("Move", |s| s.move_clips(&ids, delta, video_tracks, audio_tracks).map(|_| ()).map_err(|e| e.to_string()));
            }
            Message::TrimClips { ids, edge, delta, ripple } => {
                let label = if ripple { "Ripple Trim" } else { "Trim" };
                self.edit(label, |s| {
                    if ripple { s.ripple_trim(&ids, edge, delta) } else { s.trim(&ids, edge, delta) };
                    Ok(())
                });
            }
            Message::SlipClips { ids, delta } => {
                self.edit("Slip", |s| {
                    s.slip(&ids, delta);
                    Ok(())
                });
            }
            Message::Razor { at, tracks } => {
                self.edit("Razor", |s| {
                    s.razor(at, &tracks);
                    Ok(())
                });
            }
            Message::AddEdit { all } => {
                let at = self.playhead;
                let tracks: Vec<TrackRef> = self
                    .sequence
                    .track_refs()
                    .into_iter()
                    .filter(|&r| all || self.sequence.track(r).targeted)
                    .collect();
                self.edit(if all { "Add Edit to All Tracks" } else { "Add Edit" }, |s| {
                    s.razor(at, &tracks);
                    Ok(())
                });
            }
            Message::Insert | Message::Overwrite => {
                let insert = matches!(message, Message::Insert);
                let Some((video, audio)) = self.source_clips() else {
                    self.notice = Some("Open a clip in the Source monitor (or select one in the Project panel) first".into());
                    return;
                };
                let at = self.seq_in.unwrap_or(self.playhead);
                let length = video.as_ref().or(audio.as_ref()).map_or(0, |c| c.duration);
                let ok = self.edit(if insert { "Insert" } else { "Overwrite" }, |s| {
                    let r = if insert { s.insert(at, video.as_ref(), audio.as_ref()) } else { s.overwrite(at, video.as_ref(), audio.as_ref()) };
                    r.map(|_| ()).map_err(|e| e.to_string())
                });
                if ok {
                    // Like Premiere: the playhead lands after the new clip, marks are used up.
                    self.seq_in = None;
                    self.seq_out = None;
                    self.set_playhead(at + length);
                }
            }
            Message::Lift | Message::Extract => {
                let Some((a, b)) = self.marked_range() else {
                    self.notice = Some("Mark an In and Out in the timeline (I and O) first".into());
                    return;
                };
                let extract = matches!(message, Message::Extract);
                let ok = self.edit(if extract { "Extract" } else { "Lift" }, |s| {
                    if extract {
                        s.extract(a, b);
                        Ok(())
                    } else {
                        s.lift(a, b).map_err(|e| e.to_string())
                    }
                });
                if ok {
                    self.seq_in = None;
                    self.seq_out = None;
                    self.set_playhead(a);
                }
            }
            Message::DeleteSelection { ripple } => {
                if self.selection.is_empty() {
                    return;
                }
                let ids = self.selection.clone();
                if ripple {
                    self.edit("Ripple Delete", |s| s.ripple_delete(&ids).map_err(|e| e.to_string()));
                } else {
                    self.edit("Clear", |s| {
                        s.delete(&ids);
                        Ok(())
                    });
                }
            }
            Message::Undo => match self.history.undo(&mut self.sequence) {
                Some(label) => {
                    self.notice = Some(format!("Undo {label}"));
                    self.after_change();
                }
                None => self.notice = Some("Nothing to undo".into()),
            },
            Message::Redo => match self.history.redo(&mut self.sequence) {
                Some(label) => {
                    self.notice = Some(format!("Redo {label}"));
                    self.after_change();
                }
                None => self.notice = Some("Nothing to redo".into()),
            },
            Message::ToggleTrack(r, what) => {
                let t = self.sequence.track_mut(r);
                match what {
                    TrackToggle::Lock => t.locked = !t.locked,
                    TrackToggle::Target => t.targeted = !t.targeted,
                    TrackToggle::Enabled => t.enabled = !t.enabled,
                    TrackToggle::Solo => t.solo = !t.solo,
                }
                self.sync_program();
            }
            Message::ScrollTimeline(px) => {
                let dt = from_seconds((px / self.px_per_second) as f64);
                self.scroll = (self.scroll + dt).max(0);
            }
            Message::ZoomTimeline { factor, anchor_x } => {
                // Zoom around the pointer, or the playhead for keyboard zoom.
                let anchor_x = if anchor_x.is_finite() {
                    anchor_x
                } else {
                    (to_seconds(self.playhead - self.scroll) as f32 * self.px_per_second).clamp(0.0, self.timeline_width)
                };
                let anchor_t = self.scroll + from_seconds((anchor_x / self.px_per_second) as f64);
                self.px_per_second = (self.px_per_second * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                self.scroll = (anchor_t - from_seconds((anchor_x / self.px_per_second) as f64)).max(0);
            }
            Message::ZoomToFit => {
                let d = to_seconds(self.sequence.duration().max(jd_timeline::SECOND * 10));
                self.px_per_second = ((self.timeline_width * 0.95) / d as f32).clamp(MIN_ZOOM, MAX_ZOOM);
                self.scroll = 0;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{key, key_with, settle, with_clip};
    use crate::source::GoTo;
    use iced::keyboard::Modifiers;
    use jd_timeline::TrackRef;

    fn frames(app: &Editor, t: Time) -> i64 {
        app.sequence.rate.to_frames(t)
    }

    fn spans(app: &Editor, r: TrackRef) -> Vec<(i64, i64)> {
        app.sequence.track(r).clips.iter().map(|c| (frames(app, c.start), frames(app, c.end()))).collect()
    }

    #[test]
    fn source_in_out_inserts_linked_video_and_audio_and_undoes() {
        let Some((mut app, _)) = with_clip() else { return };
        // Mark frames 10..=29 in the source.
        let _ = app.update(Message::GoTo(GoTo::Start));
        for _ in 0..10 {
            let _ = app.update(Message::Step(1));
        }
        settle(&mut app, "step", |a| (a.source.status.position * 25.0).round() == 10.0);
        let _ = app.update(key("i"));
        let _ = app.update(Message::Scrub(29.0 / 49.0));
        settle(&mut app, "scrub", |a| (a.source.status.position * 25.0).round() == 29.0);
        let _ = app.update(key("o"));

        let _ = app.update(key("."));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 20)]);
        assert_eq!(spans(&app, TrackRef::audio(0)), [(0, 20)]);
        assert_eq!(app.sequence.video[0].clips[0].source_in, app.sequence.rate.from_frames(10));
        assert_eq!(frames(&app, app.playhead), 20, "playhead moves past the new clip");

        // Insert the same range at frame 5: everything after opens up.
        let _ = app.update(Message::SetPlayhead(app.sequence.rate.from_frames(5)));
        let _ = app.update(key(","));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 5), (5, 25), (25, 40)]);

        // Program monitor shows the clip under the playhead.
        settle(&mut app, "program frame", |a| a.program.showing);

        let _ = app.update(key_with("z", Modifiers::CTRL));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 20)]);
        let _ = app.update(key_with("z", Modifiers::CTRL | Modifiers::SHIFT));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 5), (5, 25), (25, 40)]);
    }

    #[test]
    fn whole_bin_clip_goes_in_without_the_source_monitor() {
        let Some((mut app, id)) = with_clip() else { return };
        app.source.item = None;
        app.selected_media = Some(id);
        let _ = app.update(Message::Overwrite);
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 50)]);
    }

    #[test]
    fn timeline_keys_mark_lift_extract_and_ripple_delete() {
        let Some((mut app, id)) = with_clip() else { return };
        app.source.item = None;
        app.selected_media = Some(id);
        let _ = app.update(Message::Overwrite);
        let _ = app.update(Message::Overwrite);
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 50), (50, 100)]);

        // Up/Down walk edit points.
        let _ = app.update(Message::EditPoint(-1));
        assert_eq!(frames(&app, app.playhead), 50);
        let _ = app.update(Message::EditPoint(-1));
        assert_eq!(frames(&app, app.playhead), 0);

        // Mark 10..19 (Out includes the frame under the playhead) and extract.
        let _ = app.update(Message::SetPlayhead(app.sequence.rate.from_frames(10)));
        let _ = app.update(key("i"));
        let _ = app.update(Message::SetPlayhead(app.sequence.rate.from_frames(19)));
        let _ = app.update(key("o"));
        let _ = app.update(key("'"));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 10), (10, 40), (40, 90)]);
        assert_eq!(frames(&app, app.playhead), 10);

        // Select the middle clip (and its audio) and ripple delete it.
        let mid = app.sequence.video[0].clips[1].id;
        let _ = app.update(Message::Select { id: mid, add: false });
        assert_eq!(app.selection.len(), 2, "linked audio selected too");
        let _ = app.update(Message::DeleteSelection { ripple: true });
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 10), (10, 60)]);
        assert_eq!(spans(&app, TrackRef::audio(0)), [(0, 10), (10, 60)]);
        assert!(app.selection.is_empty());

        // Ctrl+K cuts targeted tracks at the playhead.
        let _ = app.update(Message::SetPlayhead(app.sequence.rate.from_frames(30)));
        let _ = app.update(key_with("k", Modifiers::CTRL));
        assert_eq!(spans(&app, TrackRef::video(0)), [(0, 10), (10, 30), (30, 60)]);
    }

    #[test]
    fn space_plays_the_timeline_to_the_end_and_edits_stop_it() {
        let Some((mut app, id)) = with_clip() else { return };
        app.source.item = None;
        app.selected_media = Some(id);
        let _ = app.update(Message::Overwrite);
        let _ = app.update(Message::SetPlayhead(0));
        let _ = app.update(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Space),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Space),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::empty(),
            text: None,
            repeat: false,
        }.into_message());
        assert!(app.program.is_playing());
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = app.update(Message::Tick);
        assert!(app.playhead > 0, "the playhead follows playback");
        // An edit stops playback where it got to.
        let _ = app.update(key_with("k", Modifiers::CTRL));
        assert!(!app.program.is_playing());
        let cut = app.playhead;
        assert_eq!(app.sequence.video[0].clips[0].end(), cut, "Add Edit cut at the stopped position");
        // Play again and let it run to the end (2 s at the tests' 2x).
        let _ = app.update(Message::TogglePlay);
        settle(&mut app, "end of playback", |a| !a.program.is_playing());
        assert_eq!(app.playhead, app.sequence.duration());
    }

    trait IntoMessage {
        fn into_message(self) -> Message;
    }

    impl IntoMessage for iced::keyboard::Event {
        fn into_message(self) -> Message {
            Message::Key(self)
        }
    }

    #[test]
    fn lift_without_marks_explains_itself() {
        let (mut app, _) = crate::app::Editor::new();
        let _ = app.update(key(";"));
        assert!(app.notice.as_deref().unwrap().contains("Mark an In and Out"));
    }

    #[test]
    fn zoom_keeps_the_anchor_still() {
        let (mut app, _) = crate::app::Editor::new();
        app.px_per_second = 10.0;
        app.scroll = jd_timeline::SECOND * 5;
        // Time under x=100 is 5 + 10 = 15 s; after zooming it's still under x=100.
        let _ = app.update(Message::ZoomTimeline { factor: 2.0, anchor_x: 100.0 });
        assert_eq!(app.px_per_second, 20.0);
        assert_eq!(app.scroll, jd_timeline::SECOND * 10);
    }
}
