//! The Assets tab: the document's components with a picture each. Click one to place an
//! instance in the middle of the view (or of the selected frame).

use iced::alignment::Vertical;
use iced::widget::{button, column, container, image, row, scrollable, space, text, text_input};
use iced::{Element, Fill, Padding};
use jd_ui::widget::panel_header;
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::NodeKind;

use super::COMPONENT;
use crate::app::{Blobr, Message};

/// One component row: picture, name, what it is.
fn item<'a>(doc: &'a jd_vector::Document, id: jd_vector::NodeId, thumb: Option<&image::Handle>, on: Message) -> Option<Element<'a, Message>> {
    let n = doc.node(id)?;
    let picture: Element<'_, Message> = match thumb {
        Some(h) => container(image(h.clone()).width(40).height(40).content_fit(iced::ContentFit::Contain)).center(44).into(),
        None => container(icon(Icon::Component, 16.0, COMPONENT)).center(44).into(),
    };
    let variants = match n.kind {
        NodeKind::ComponentSet { .. } => format!("{} variants", doc.children(id).len()),
        _ => "Component".into(),
    };
    let label = column![text(n.name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY), text(variants).size(10).color(TEXT_MUTED)].spacing(2);
    Some(button(row![picture, label].spacing(8).align_y(Vertical::Center)).width(Fill).padding(Padding::from([2, 6])).style(style::row(false)).on_press(on).into())
}

fn heading<'a>(label: String) -> Element<'a, Message> {
    container(text(label).size(10).color(TEXT_MUTED)).padding(Padding { top: 8.0, bottom: 2.0, left: 6.0, right: 0.0 }).into()
}

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let filter = app.asset_filter.to_lowercase();
    let matches = |d: &jd_vector::Document, id| filter.is_empty() || d.node(id).is_some_and(|n| n.name.to_lowercase().contains(&filter));
    let mut list = column![].spacing(2);
    let mut count = 0;
    let mut last_page = None;
    // Libraries' components, then this file's.
    for (li, lib) in app.libraries.iter().enumerate() {
        list = list.push(
            row![
                text(format!("Library: {}", lib.name)).size(10).color(TEXT_SECONDARY),
                space::horizontal(),
                button(icon(Icon::Close, 10.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::RemoveLibrary(li)),
            ]
            .align_y(Vertical::Center)
            .padding(Padding { top: 8.0, bottom: 2.0, left: 6.0, right: 0.0 }),
        );
        for (id, _) in crate::app::placeable_components(&lib.doc) {
            if matches(&lib.doc, id)
                && let Some(e) = item(&lib.doc, id, lib.thumbnails.get(&id), Message::InsertLibraryComponent(li, id))
            {
                count += 1;
                list = list.push(e);
            }
        }
    }
    for (id, page) in app.components() {
        if !matches(doc, id) {
            continue;
        }
        if last_page != Some(page) {
            last_page = Some(page);
            list = list.push(heading(doc.node(page).map_or(String::new(), |p| p.name.clone())));
        }
        if let Some(e) = item(doc, id, app.thumbnails.get(&id), Message::InsertComponent(id)) {
            count += 1;
            list = list.push(e);
        }
    }
    let body: Element<'_, Message> = if count == 0 {
        container(
            text(if filter.is_empty() { "No components yet. Select a layer and press Ctrl+Alt+K to make one." } else { "No components match." })
                .size(TEXT_SIZE_SMALL)
                .color(TEXT_SECONDARY),
        )
        .padding(12)
        .into()
    } else {
        scrollable(container(list).padding(Padding::from([0, 6]))).height(Fill).into()
    };
    column![
        panel_header(&["Layers", "Assets"], 1, Message::LeftTab),
        container(
            text_input("Search assets", &app.asset_filter)
                .on_input(Message::AssetFilter)
                .size(TEXT_SIZE_SMALL)
                .padding(Padding::from([4, 8]))
        )
        .padding(8),
        body,
        container(button(text("+ Add library...").size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 10])).style(style::ghost(false)).on_press(Message::AddLibrary))
            .padding(8),
    ]
    .into()
}
