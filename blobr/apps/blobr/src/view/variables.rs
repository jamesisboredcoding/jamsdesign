//! The Local variables dialog: collections on the left, their variables in a table with a
//! column per mode. Values apply on Enter.

use iced::alignment::Vertical;
use iced::widget::{button, checkbox, column, container, mouse_area, row, scrollable, space, text};
use iced::{Border, Color, Element, Fill, Padding};
use jd_ui::widget::{bold, hline};
use jd_ui::{Icon, icon, style, tokens::*};
use jd_vector::VarValue;

use super::design::{field, swatch};
use crate::app::{Blobr, Field, Message};

const NAME_W: f32 = 180.0;
const MODE_W: f32 = 170.0;

pub fn view(app: &Blobr) -> Element<'_, Message> {
    let tokens = app.doc().tokens();
    let mut collections = column![].spacing(2);
    for (&id, c) in &tokens.collections {
        let current = app.var_collection == Some(id);
        collections = collections.push(
            button(text(c.name.clone()).size(TEXT_SIZE_SMALL).color(TEXT_PRIMARY))
                .width(Fill)
                .padding(Padding::from([5, 10]))
                .style(style::row(current))
                .on_press(Message::SelectCollection(id)),
        );
    }
    collections = collections.push(
        button(text("+ Collection").size(TEXT_SIZE_SMALL)).padding(Padding::from([5, 10])).style(style::ghost(false)).on_press(Message::AddCollection),
    );

    let table: Element<'_, Message> = match app.var_collection.and_then(|c| tokens.collections.get(&c).map(|col| (c, col))) {
        None => container(text("Make a collection to start: variables live in collections, with a value per mode.").size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY))
            .padding(16)
            .into(),
        Some((cid, col)) => {
            let mut head = row![text("Name").size(10).color(TEXT_MUTED).width(NAME_W)].spacing(8);
            for m in &col.modes {
                head = head.push(text(m.clone()).size(10).color(TEXT_MUTED).width(MODE_W));
            }
            head = head.push(button(text("+ Mode").size(10)).padding(Padding::from([2, 6])).style(style::ghost(false)).on_press(Message::AddMode));
            let mut rows = column![head, hline(BORDER_SUBTLE)].spacing(6);
            for (&vid, var) in tokens.variables.iter().filter(|(_, v)| v.collection == cid) {
                let mut r = row![container(field(app, "", Field::VarName(vid), || var.name.clone())).width(NAME_W)].spacing(8).align_y(Vertical::Center);
                let first = var.values.get(&col.modes[0]).cloned();
                for m in &col.modes {
                    let v = var.values.get(m).cloned().or_else(|| first.clone());
                    let cell: Element<'_, Message> = match v {
                        Some(VarValue::Bool(b)) => {
                            let (mode, id) = (m.clone(), vid);
                            checkbox(b).on_toggle(move |b| Message::SetVarBool(id, mode.clone(), b)).size(14).into()
                        }
                        Some(VarValue::Color(c)) => {
                            let hex = c.to_hex();
                            row![swatch(&hex), field(app, "", Field::VarValue(vid, m.clone()), move || hex)].spacing(6).align_y(Vertical::Center).into()
                        }
                        Some(VarValue::Number(n)) => field(app, "", Field::VarValue(vid, m.clone()), move || format!("{n}")),
                        Some(VarValue::Text(t)) => field(app, "", Field::VarValue(vid, m.clone()), move || t),
                        Some(VarValue::Alias(a)) => {
                            let name = tokens.variables.get(&a).map_or("?".into(), |v| v.name.clone());
                            text(format!("→ {name}")).size(TEXT_SIZE_SMALL).color(TEXT_SECONDARY).into()
                        }
                        None => space().into(),
                    };
                    r = r.push(container(cell).width(MODE_W));
                }
                r = r.push(button(icon(Icon::Minus, 12.0, ICON)).padding(3).style(style::ghost(false)).on_press(Message::DeleteToken(vid)));
                rows = rows.push(r);
            }
            let add = |label: &'static str, kind: &'static str| {
                button(text(label).size(TEXT_SIZE_SMALL)).padding(Padding::from([4, 8])).style(style::ghost(false)).on_press(Message::AddVariable(kind))
            };
            rows = rows.push(row![add("+ Color", "color"), add("+ Number", "number"), add("+ Text", "text"), add("+ Boolean", "bool")].spacing(6));
            scrollable(container(rows).padding(12)).height(Fill).into()
        }
    };

    let header = row![
        text("Local variables").size(TEXT_SIZE).font(bold()).color(TEXT_PRIMARY),
        space::horizontal(),
        button(icon(Icon::Close, 12.0, ICON)).padding(4).style(style::ghost(false)).on_press(Message::ShowVariables(false)),
    ]
    .align_y(Vertical::Center);
    let card = container(
        column![
            container(header).padding(Padding::from([10, 14])),
            hline(BORDER_DEFAULT),
            row![container(scrollable(collections)).width(170).height(Fill).padding(8), iced::widget::rule::vertical(1), table].height(Fill),
        ],
    )
    .width(Fill)
    .max_width(980)
    .height(560)
    .style(|_| container::Style {
        background: Some(BG_PANEL.into()),
        border: Border { radius: 6.0.into(), width: 1.0, color: BORDER_DEFAULT },
        ..Default::default()
    });
    iced::widget::opaque(
        mouse_area(container(card).center(Fill).padding(40).style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
            ..Default::default()
        }))
        .on_press(Message::Noop),
    )
}
