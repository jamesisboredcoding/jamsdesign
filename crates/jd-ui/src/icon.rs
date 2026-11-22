use std::sync::LazyLock;

use iced::Color;
use iced::widget::svg::{self, Handle, Svg};

macro_rules! icons {
    ($($variant:ident => $file:literal),* $(,)?) => {
        /// Every icon shipped in `crates/jd-ui/icons`. They're white line icons; tint them with
        /// [`icon`]'s colour argument.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon { $($variant),* }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$variant),*];

            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("../icons/", $file, ".svg"))),*
                }
            }
        }
    };
}

icons! {
    Move => "move", Marquee => "marquee", Lasso => "lasso", Wand => "wand", Crop => "crop",
    EyeOff => "eye-off", Burn => "burn", SpotHeal => "spot-heal", AlignLeft => "align-left", AlignCenter => "align-center", AlignRight => "align-right", EllipseShape => "ellipse-shape",
    Eyedropper => "eyedropper", Heal => "heal", Brush => "brush", Stamp => "stamp",
    Eraser => "eraser", Gradient => "gradient", Dodge => "dodge", Pen => "pen", Type => "type",
    Select => "select", Shape => "shape", Hand => "hand", Zoom => "zoom", Eye => "eye",
    Chevron => "chevron", Lock => "lock", Plus => "plus", Trash => "trash", Folder => "folder",
    Link => "link", Adjust => "adjust", Mask => "mask", History => "history", Layers => "layers",
    Close => "close", Menu => "menu", Swap => "swap", Home => "home", Share => "share", Fx => "fx",
    Brushes => "brushes", Info => "info", Corner => "corner", Minimize => "minimize",
    Maximize => "maximize", Restore => "restore", Fullscreen => "fullscreen",
    ExitFullscreen => "exit-fullscreen", Image => "image", MarqueeEllipse => "marquee-ellipse",
    PolygonLasso => "polygon-lasso",
    // Video (Cinematica)
    MarkIn => "mark-in", MarkOut => "mark-out", GoToIn => "go-to-in", StepBack => "step-back",
    Play => "play", Pause => "pause", StepForward => "step-forward", GoToOut => "go-to-out",
    Insert => "insert", Overwrite => "overwrite", Lift => "lift", Extract => "extract",
    Camera => "camera", Magnet => "magnet", Marker => "marker", Razor => "razor",
    Ripple => "ripple", Slip => "slip", TrackSelect => "track-select", Wrench => "wrench",
    Mic => "mic", Music => "music", List => "list", Grid => "grid", Film => "film",
}

/// Handles are built once: creating one hashes the SVG bytes, which we don't want per frame.
static HANDLES: LazyLock<Vec<Handle>> = LazyLock::new(|| {
    Icon::ALL
        .iter()
        .map(|i| Handle::from_memory(i.bytes()))
        .collect()
});

impl Icon {
    pub fn handle(self) -> Handle {
        HANDLES[self as usize].clone()
    }
}

pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Svg<'a> {
    Svg::new(icon.handle())
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}
