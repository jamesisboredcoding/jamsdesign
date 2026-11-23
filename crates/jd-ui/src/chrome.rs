//! Window chrome shared by the suite's apps: our own title bar (the windows have no system
//! decorations), menu-bar dropdown switching, 5px resize edges and window-state tracking.
//!
//! The app keeps a [`WindowChrome`], forwards [`Event`]s to [`WindowChrome::update`], and
//! builds the bar with [`title_bar`]. Everything that isn't generic (the menus' contents,
//! the actions next to the window controls, what closing means) stays in the app.

use iced::alignment::Vertical;
use iced::mouse::Interaction;
use iced::widget::{button, column, container, mouse_area, pin, row, space, stack, text};
use iced::window::{self, Direction};
use iced::{Element, Fill, Length, Padding, Point, Size, Subscription, Task};

use crate::icon::{Icon, icon};
use crate::style;
use crate::tokens::*;
use crate::widget::{MenuEntry, menu, menu_height};

/// Height of the title bar. Dropdowns open just below it.
pub const TITLE_BAR_HEIGHT: f32 = 30.0;

#[derive(Debug, Clone)]
pub enum Event {
    /// The window id, from [`WindowChrome::new`]'s task.
    Opened(Option<window::Id>),
    TitlePressed,
    TitleMoved(Point),
    TitleReleased,
    /// Pointer over menu title `i`, at this position inside the title.
    MenuPointer(usize, Point),
    /// Menu title `i` clicked: open its dropdown, or close it if it's already open.
    MenuTitle(usize),
    CloseMenu,
    /// A window was resized (apps with more than one window only follow their main one).
    Resized(window::Id, Size),
    Maximized(bool),
    ModeChanged(window::Mode),
    ResizeWindow(Direction),
    Minimize,
    ToggleMaximize,
    ToggleFullscreen,
}

#[derive(Debug)]
pub struct WindowChrome {
    window: Option<window::Id>,
    pub size: Size,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The open title-bar dropdown.
    pub open_menu: Option<usize>,
    /// Pressed but not yet dragged: dragging starts on the first move, so a double-click can
    /// still maximise without fighting a compositor move.
    title_pressed: bool,
    /// Menu title under the pointer and the pointer's x inside it, waiting for the title
    /// bar's window-space position to arrive (the inner mouse area reports first).
    hovered_menu: Option<(usize, f32)>,
    /// Left edge of each menu title in window coordinates, measured as the pointer passes.
    menu_left: Vec<f32>,
}

impl WindowChrome {
    pub fn new(menu_count: usize, size: Size) -> (Self, Task<Event>) {
        let chrome = Self {
            window: None,
            size,
            maximized: false,
            fullscreen: false,
            open_menu: None,
            title_pressed: false,
            hovered_menu: None,
            menu_left: vec![0.0; menu_count],
        };
        (chrome, window::oldest().map(Event::Opened))
    }

    pub fn window(&self) -> Option<window::Id> {
        self.window
    }

    /// Where the open dropdown appears, in window coordinates.
    pub fn menu_anchor(&self) -> Option<(usize, Point)> {
        self.open_menu.map(|i| (i, Point::new(self.menu_left[i], TITLE_BAR_HEIGHT)))
    }

    pub fn update(&mut self, event: Event) -> Task<Event> {
        match event {
            Event::Opened(id) => self.window = id,
            Event::TitlePressed => {
                self.title_pressed = true;
                self.open_menu = None;
            }
            Event::TitleReleased => self.title_pressed = false,
            Event::MenuPointer(i, p) => self.hovered_menu = Some((i, p.x)),
            Event::TitleMoved(p) => {
                if let Some((i, local_x)) = self.hovered_menu.take() {
                    self.menu_left[i] = p.x - local_x;
                    // With a menu open, hovering another title switches to it.
                    if self.open_menu.is_some() {
                        self.open_menu = Some(i);
                    }
                }
                if std::mem::take(&mut self.title_pressed) {
                    return self.with_window(window::drag);
                }
            }
            Event::MenuTitle(i) => {
                self.open_menu = if self.open_menu == Some(i) { None } else { Some(i) };
            }
            Event::CloseMenu => self.open_menu = None,
            // Maximised/fullscreen are re-queried on every resize so they stay right when
            // KWin maximises or snaps the window by other means.
            Event::Resized(id, size) => {
                if self.window.is_some_and(|w| w != id) {
                    return Task::none();
                }
                self.size = size;
                return self.with_window(|id| {
                    Task::batch([
                        window::is_maximized(id).map(Event::Maximized),
                        window::mode(id).map(Event::ModeChanged),
                    ])
                });
            }
            Event::Maximized(m) => self.maximized = m,
            Event::ModeChanged(mode) => self.fullscreen = mode == window::Mode::Fullscreen,
            Event::ResizeWindow(dir) => return self.with_window(move |id| window::drag_resize(id, dir)),
            Event::Minimize => return self.with_window(|id| window::minimize(id, true)),
            Event::ToggleMaximize => {
                self.title_pressed = false;
                return self.with_window(window::toggle_maximize);
            }
            Event::ToggleFullscreen => {
                let mode = if self.fullscreen { window::Mode::Windowed } else { window::Mode::Fullscreen };
                return self.with_window(move |id| window::set_mode(id, mode));
            }
        }
        Task::none()
    }

    /// Closes the window (the app decides when: after asking about unsaved work).
    pub fn close<T: Send + 'static>(&self) -> Task<T> {
        self.window.map_or_else(Task::none, window::close)
    }

    pub fn subscription() -> Subscription<Event> {
        window::resize_events().map(|(id, size)| Event::Resized(id, size))
    }

    fn with_window(&self, f: impl FnOnce(window::Id) -> Task<Event>) -> Task<Event> {
        self.window.map_or_else(Task::none, f)
    }
}

/// The title bar: app logo (SVG bytes), menu titles, centred title, the app's `actions` (workspace
/// switcher, export...), then minimise / maximise / close. Empty space drags the window and
/// double-click maximises.
pub fn title_bar<'a, M: Clone + 'a>(
    chrome: &WindowChrome,
    logo: &'static [u8],
    menus: &'a [&'a str],
    title: String,
    actions: impl Into<Element<'a, M>>,
    on_close: M,
    map: impl Fn(Event) -> M + Clone + 'a,
) -> Element<'a, M> {
    // The app's logo, in its own colours (not tinted like the UI icons).
    let badge = iced::widget::svg(iced::widget::svg::Handle::from_memory(logo)).width(22).height(22);

    let mut titles = row![badge, space().width(8)].align_y(Vertical::Center).spacing(2);
    for (i, m) in menus.iter().enumerate() {
        titles = titles.push(
            mouse_area(
                button(text(*m).size(TEXT_SIZE))
                    .padding(Padding::from([4, 8]))
                    .style(style::ghost(chrome.open_menu == Some(i)))
                    .on_press(Event::MenuTitle(i)),
            )
            .on_move(move |p| Event::MenuPointer(i, p)),
        );
    }
    let titles = Element::from(titles).map(map.clone());

    fn control<'a, T: 'a>(ic: Icon, danger: bool) -> button::Button<'a, T> {
        button(container(icon(ic, 14.0, ICON)).center(Fill))
            .width(44)
            .height(TITLE_BAR_HEIGHT)
            .padding(0)
            .style(style::window_control(danger))
    }
    let controls = row![
        Element::from(control(Icon::Minimize, false).on_press(Event::Minimize)).map(map.clone()),
        Element::from(
            control(if chrome.maximized { Icon::Restore } else { Icon::Maximize }, false)
                .on_press(Event::ToggleMaximize)
        )
        .map(map.clone()),
        control(Icon::Close, true).on_press(on_close),
    ];

    let bar = container(
        row![
            container(titles).padding(Padding::ZERO.left(10)),
            space::horizontal(),
            text(title).size(TEXT_SIZE_SMALL).color(TEXT_MUTED),
            space::horizontal(),
            actions.into(),
            space().width(12),
            controls,
        ]
        .align_y(Vertical::Center),
    )
    .height(TITLE_BAR_HEIGHT)
    .align_y(Vertical::Center)
    .style(style::fill(BG_PANEL_HEADER));

    // Buttons inside capture their own clicks, so presses that reach here are on empty space.
    mouse_area(bar)
        .on_press(map(Event::TitlePressed))
        .on_move({
            let map = map.clone();
            move |p| map(Event::TitleMoved(p))
        })
        .on_release(map(Event::TitleReleased))
        .on_double_click(map(Event::ToggleMaximize))
        .into()
}

/// Invisible grab strips along the window edges. Without system decorations the compositor
/// gives us no resize border, so we start the resize ourselves. Leave them out while the
/// window is maximised or fullscreen.
pub fn resize_edges<'a, M: Clone + 'a>(map: impl Fn(Event) -> M + 'a) -> Element<'a, M> {
    const EDGE: f32 = 5.0;
    let grip = |w: Length, h: Length, dir: Direction, cursor: Interaction| -> Element<'a, Event> {
        mouse_area(space().width(w).height(h))
            .interaction(cursor)
            .on_press(Event::ResizeWindow(dir))
            .into()
    };
    let e = Length::Fixed(EDGE);
    Element::from(column![
        row![
            grip(e, e, Direction::NorthWest, Interaction::ResizingDiagonallyDown),
            grip(Fill, e, Direction::North, Interaction::ResizingVertically),
            grip(e, e, Direction::NorthEast, Interaction::ResizingDiagonallyUp),
        ],
        row![
            grip(e, Fill, Direction::West, Interaction::ResizingHorizontally),
            space().width(Fill).height(Fill),
            grip(e, Fill, Direction::East, Interaction::ResizingHorizontally),
        ]
        .height(Fill),
        row![
            grip(e, e, Direction::SouthWest, Interaction::ResizingDiagonallyUp),
            grip(Fill, e, Direction::South, Interaction::ResizingVertically),
            grip(e, e, Direction::SouthEast, Interaction::ResizingDiagonallyDown),
        ],
    ])
    .map(map)
}

/// A popup menu at `at` (window coordinates), kept inside the window, over a click-catcher
/// that sends `on_dismiss`. For title-bar dropdowns pass `below_title_bar`: the catcher then
/// leaves the title bar uncovered, so clicking or hovering another title switches menus.
/// `open_sub` and `on_hover` drive submenus as in [`menu`]; an app without submenus can pass
/// `None` and a closure returning a no-op message.
#[allow(clippy::too_many_arguments)]
pub fn popup<'a, M: Clone + 'a>(
    entries: Vec<MenuEntry<'a, M>>,
    width: f32,
    at: Point,
    window: Size,
    below_title_bar: bool,
    on_dismiss: M,
    open_sub: Option<usize>,
    on_hover: impl Fn(Option<usize>) -> M + 'a,
) -> Element<'a, M> {
    let height = menu_height(&entries);
    let x = at.x.min(window.width - width - 4.0).max(0.0);
    let y = if at.y + height > window.height { (at.y - height).max(0.0) } else { at.y };

    let catcher = mouse_area(space().width(Fill).height(Fill))
        .on_press(on_dismiss.clone())
        .on_right_press(on_dismiss);
    let catcher: Element<'a, M> = if below_title_bar {
        column![space().height(TITLE_BAR_HEIGHT), catcher].into()
    } else {
        catcher.into()
    };
    let view = menu(entries, width, open_sub, on_hover);
    let mut layers = stack![catcher, pin(view.main).x(x).y(y)];
    if let Some((dy, sub)) = view.sub {
        // Beside the menu, or on its left when there's no room on the right.
        let sx = if x + 2.0 * width + 4.0 <= window.width { x + width - 4.0 } else { (x - width + 4.0).max(0.0) };
        layers = layers.push(pin(sub).x(sx).y(y + dy));
    }
    layers.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hovering_another_title_switches_the_open_menu() {
        let (mut c, _) = WindowChrome::new(4, Size::new(800.0, 600.0));
        // Pointer 5px into title 1, which sits at x=80 in the window.
        let _ = c.update(Event::MenuPointer(1, Point::new(5.0, 10.0)));
        let _ = c.update(Event::TitleMoved(Point::new(85.0, 10.0)));
        assert_eq!(c.menu_anchor(), None);
        let _ = c.update(Event::MenuTitle(1));
        assert_eq!(c.menu_anchor(), Some((1, Point::new(80.0, TITLE_BAR_HEIGHT))));
        // While open, hovering title 2 switches to it.
        let _ = c.update(Event::MenuPointer(2, Point::new(3.0, 10.0)));
        let _ = c.update(Event::TitleMoved(Point::new(130.0, 10.0)));
        assert_eq!(c.menu_anchor(), Some((2, Point::new(127.0, TITLE_BAR_HEIGHT))));
        // Clicking the open title closes it.
        let _ = c.update(Event::MenuTitle(2));
        assert_eq!(c.open_menu, None);
        // Closed: hovering doesn't open anything.
        let _ = c.update(Event::MenuPointer(3, Point::new(1.0, 1.0)));
        let _ = c.update(Event::TitleMoved(Point::new(200.0, 1.0)));
        assert_eq!(c.open_menu, None);
    }

    #[test]
    fn pressing_the_title_bar_closes_the_menu() {
        let (mut c, _) = WindowChrome::new(2, Size::new(800.0, 600.0));
        let _ = c.update(Event::MenuTitle(0));
        let _ = c.update(Event::TitlePressed);
        assert_eq!(c.open_menu, None);
    }
}
