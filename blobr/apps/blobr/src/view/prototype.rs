//! The Prototype tab: the selected layer's interactions (trigger, action, destination,
//! transition), and with nothing selected, the frames to present from.

use iced::alignment::Vertical;
use iced::widget::{button, column, container, pick_list, row, scrollable, space, text};
use iced::{Element, Fill, Padding};
use jd_ui::widget::{bold, hline};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::{Action, Transition, Trigger};

use super::design::field;
use crate::app::{Blobr, Field, Message};

#[derive(Debug, Clone, PartialEq)]
struct Choice<T>(T, String);

impl<T> std::fmt::Display for Choice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

fn picker<'a, T: Clone + PartialEq + 'a>(options: Vec<Choice<T>>, cur: impl Fn(&T) -> bool, on: impl Fn(T) -> Message + 'a) -> Element<'a, Message> {
    let selected = options.iter().find(|c| cur(&c.0)).cloned();
    pick_list(options, selected, move |c: Choice<T>| on(c.0))
        .text_size(TEXT_SIZE_SMALL)
        .padding(Padding::from([3, 6]))
        .style(style::pick_list)
        .width(Fill)
        .into()
}

fn line<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    row![text(label).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).width(78), control].align_y(Vertical::Center).into()
}

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let frames: Vec<Choice<jd_vector::NodeId>> = app.top_frames().into_iter().filter_map(|f| Some(Choice(f, doc.node(f)?.name.clone()))).collect();
    let present = |label: &'static str, from: Option<jd_vector::NodeId>| {
        button(row![icon(Icon::Play, 12.0, TEXT_PRIMARY), text(label).size(TEXT_SIZE_SMALL)].spacing(6).align_y(Vertical::Center))
            .padding(Padding::from([4, 10]))
            .style(style::primary)
            .on_press(Message::Present(from))
    };
    let [id] = app.selection[..] else {
        let flows = app.flows();
        let mut col = column![text("Flows").size(TEXT_SIZE_SMALL).font(bold()).color(TEXT_PRIMARY)].spacing(8);
        if flows.is_empty() {
            col = col.push(text("Select a top-level frame and add a starting point to save a flow here.").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY));
        }
        for (f, name) in flows {
            col = col.push(
                row![icon(Icon::Play, 10.0, TEXT_SECONDARY), text(name).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY), space::horizontal(), button(icon(Icon::Play, 12.0, ICON)).padding(4).style(style::primary).on_press(Message::Present(Some(f)))]
                    .spacing(6)
                    .align_y(Vertical::Center),
            );
        }
        col = col.push(hline(BG_PANEL));
        col = col.push(text("Select a layer and add an interaction to link frames. Present from any frame:").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY));
        for f in &frames {
            col = col.push(
                row![text(f.1.clone()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY), space::horizontal(), button(icon(Icon::Play, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::Present(Some(f.0)))]
                    .align_y(Vertical::Center),
            );
        }
        if frames.is_empty() {
            col = col.push(text("No frames on this page yet.").size(10).color(TEXT_MUTED));
        }
        return scrollable(container(col).padding(12)).height(Fill).into();
    };
    let Some(n) = doc.node(id) else { return space().into() };
    let mut col = column![
        row![text("Interactions").size(TEXT_SIZE_SMALL).font(bold()).color(TEXT_PRIMARY), space::horizontal(), button(icon(Icon::Plus, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::AddReaction)]
            .align_y(Vertical::Center),
    ]
    .spacing(10);
    if n.props.reactions.is_empty() {
        col = col.push(text("Add one to make this layer do something when you present: go to another frame, open an overlay, go back...").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY));
    }
    let triggers = vec![Choice("click", "On click".into()), Choice("hover", "While hovering".into()), Choice("delay", "After delay".into())];
    let actions = vec![Choice("navigate", "Navigate to".into()), Choice("overlay", "Open overlay".into()), Choice("back", "Back".into()), Choice("url", "Open link".into())];
    let transitions = vec![
        Choice(Transition::Instant, "Instant".into()),
        Choice(Transition::Dissolve, "Dissolve".into()),
        Choice(Transition::SlideLeft, "Slide in from right".into()),
        Choice(Transition::SlideRight, "Slide in from left".into()),
        Choice(Transition::SlideUp, "Slide in from bottom".into()),
        Choice(Transition::SlideDown, "Slide in from top".into()),
        Choice(Transition::SmartAnimate, "Smart animate".into()),
    ];
    for (i, r) in n.props.reactions.iter().enumerate() {
        let trig = match r.trigger {
            Trigger::Click => "click",
            Trigger::Hover => "hover",
            Trigger::AfterDelay { .. } => "delay",
        };
        let act = match r.action {
            Action::Navigate { .. } => "navigate",
            Action::Overlay { .. } => "overlay",
            Action::Back => "back",
            Action::OpenUrl { .. } => "url",
        };
        let mut block = column![
            row![picker(triggers.clone(), |t| *t == trig, move |t| Message::SetTrigger(i, t)), button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::RemoveReaction(i))]
                .spacing(6)
                .align_y(Vertical::Center),
        ]
        .spacing(6);
        if let Trigger::AfterDelay { seconds } = r.trigger {
            block = block.push(line("Seconds", field(app, "", Field::Reaction(i, "delay"), move || format!("{seconds}"))));
        }
        block = block.push(line("Action", picker(actions.clone(), |a| *a == act, move |a| Message::SetAction(i, a))));
        match &r.action {
            Action::Navigate { to } | Action::Overlay { to } => {
                let to = *to;
                block = block.push(line("Destination", picker(frames.clone(), |f| *f == to, move |f| Message::SetTarget(i, f))));
            }
            Action::OpenUrl { url } => {
                let url = url.clone();
                block = block.push(line("Link", field(app, "", Field::Reaction(i, "url"), move || url.clone())));
            }
            Action::Back => {}
        }
        if !matches!(r.action, Action::OpenUrl { .. }) {
            let t = r.transition;
            block = block.push(line("Animation", picker(transitions.clone(), |x| *x == t, move |x| Message::SetTransition(i, x))));
            if t != Transition::Instant {
                let ms = (r.duration * 1000.0).round();
                block = block.push(line("Duration ms", field(app, "", Field::Reaction(i, "duration"), move || format!("{ms}"))));
            }
        }
        col = col.push(container(block).padding(8).style(style::fill(BG_PANEL)));
    }
    // Top-level frames can start a flow; frames can scroll what's inside them.
    if app.top_frame(id) == Some(id) {
        col = col.push(hline(BG_PANEL));
        col = col.push(match &n.props.flow {
            Some(name) => {
                let name = name.clone();
                column![
                    row![text("Flow starting point").size(TEXT_SIZE_SMALL).font(bold()).color(TEXT_PRIMARY), space::horizontal(), button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::SetFlow(false))]
                        .align_y(Vertical::Center),
                    line("Name", field(app, "", Field::FlowName, move || name.clone())),
                ]
                .spacing(6)
                .into()
            }
            None => Element::from(
                button(row![icon(Icon::Plus, 12.0, ICON), text("Add flow starting point").size(TEXT_SIZE_SMALL)].spacing(6).align_y(Vertical::Center))
                    .padding(Padding::from([4, 8]))
                    .style(style::ghost(false))
                    .on_press(Message::SetFlow(true)),
            ),
        });
    }
    if n.kind.is_frame_like() {
        use jd_vector::Overflow;
        let options = vec![
            Choice(Overflow::None, "No scrolling".into()),
            Choice(Overflow::Vertical, "Vertical".into()),
            Choice(Overflow::Horizontal, "Horizontal".into()),
            Choice(Overflow::Both, "Both directions".into()),
        ];
        let cur = n.props.overflow;
        col = col.push(line("Overflow", picker(options, move |o| *o == cur, Message::SetOverflow)));
        if cur != Overflow::None && !n.props.clip {
            col = col.push(text("Turn on Clip content for this frame so it scrolls.").size(10).color(TEXT_MUTED));
        }
    }
    col = col.push(hline(BG_PANEL));
    col = col.push(present("Present from here", app.top_frame(id)));
    scrollable(container(col).padding(12)).height(Fill).into()
}
