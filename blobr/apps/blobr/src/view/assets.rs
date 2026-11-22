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

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let doc = app.doc();
    let filter = app.asset_filter.to_lowercase();
    let mut list = column![].spacing(2);
    let mut count = 0;
    let mut last_page = None;
    for (id, page) in app.components() {
        let Some(n) = doc.node(id) else { continue };
        if !filter.is_empty() && !n.name.to_lowercase().contains(&filter) {
            continue;
        }
        if last_page != Some(page) {
            last_page = Some(page);
            let name = doc.node(page).map_or(String::new(), |p| p.name.clone());
            list = list.push(container(text(name).size(10).color(TEXT_MUTED)).padding(Padding { top: 8.0, bottom: 2.0, left: 6.0, right: 0.0 }));
        }
        count += 1;
        let picture: Element<'_, Message> = match app.thumbnails.get(&id) {
            Some(h) => container(image(h.clone()).width(40).height(40).content_fit(iced::ContentFit::Contain)).center(44).into(),
            None => container(icon(Icon::Component, 16.0, COMPONENT)).center(44).into(),
        };
        let variants = match n.kind {
            NodeKind::ComponentSet { .. } => format!("{} variants", doc.children(id).len()),
            _ => "Component".into(),
        };
        let label = column![text(n.name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY), text(variants).size(10).color(TEXT_MUTED)].spacing(2);
        list = list.push(
            button(row![picture, label].spacing(8).align_y(Vertical::Center))
                .width(Fill)
                .padding(Padding::from([2, 6]))
                .style(style::row(false))
                .on_press(Message::InsertComponent(id)),
        );
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
        space().height(0),
    ]
    .into()
}
