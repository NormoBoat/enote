use iced::{
    Background, Border, Color, Font, Theme, theme,
    widget::{button, container, text, text_editor},
};
use lucide_icons::Icon;

pub const CANVAS: Color = Color::from_rgb8(30, 30, 30);
pub const PANEL: Color = Color::from_rgb8(37, 37, 37);
pub const RAIL: Color = Color::from_rgb8(42, 42, 42);
pub const LINE: Color = Color::from_rgb8(49, 49, 49);
pub const INK: Color = Color::from_rgb8(205, 205, 205);
pub const MUTED: Color = Color::from_rgb8(143, 143, 143);
pub const DROP: Color = Color::from_rgb8(107, 154, 143);

pub fn theme() -> Theme {
    Theme::custom(
        "eNote",
        theme::Palette {
            background: CANVAS,
            text: INK,
            primary: Color::from_rgb8(107, 154, 143),
            success: Color::from_rgb8(109, 161, 126),
            warning: Color::from_rgb8(204, 174, 107),
            danger: Color::from_rgb8(199, 112, 112),
        },
    )
}

pub fn icon(symbol: Icon) -> text::Text<'static> {
    text(char::from(symbol).to_string())
        .font(Font::with_name("lucide"))
        .size(16)
}

pub fn surface(color: Color) -> container::Style {
    container::Style {
        background: Some(color.into()),
        text_color: Some(INK),
        ..Default::default()
    }
}

pub fn popup(_: &Theme) -> container::Style {
    container::Style {
        background: Some(PANEL.into()),
        text_color: Some(INK),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        },
        ..Default::default()
    }
}

pub fn tree_row(selected: bool, hovered: bool) -> container::Style {
    container::Style {
        background: (selected || hovered).then_some(LINE.into()),
        text_color: Some(if selected { INK } else { MUTED }),
        border: Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn quiet(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then_some(Background::Color(LINE)),
        text_color: if status == button::Status::Disabled {
            Color::from_rgb8(85, 85, 85)
        } else {
            MUTED
        },
        border: Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn selected(theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(LINE.into()),
        text_color: INK,
        ..quiet(theme, status)
    }
}

pub fn editor(_: &Theme, _: text_editor::Status) -> text_editor::Style {
    text_editor::Style {
        background: CANVAS.into(),
        border: Border::default(),
        placeholder: MUTED,
        value: INK,
        selection: Color::from_rgb8(55, 83, 78),
    }
}
