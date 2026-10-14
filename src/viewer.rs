use crate::{app::Message, appearance as look};
use enote::{
    buffer::BufferId,
    reader::{ReadingPosition, RenderedPage},
};
use iced::{
    Element, Fill,
    widget::{
        Space, button, column, container, image, mouse_area, row, scrollable, text, text_input,
        tooltip,
    },
};
use lucide_icons::Icon;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum ReaderAction {
    Previous,
    Next,
    ZoomIn,
    ZoomOut,
    Fit,
    PageInput(String),
    Go,
    Retry,
    Hover(bool),
    StreamScroll { request: u64, x: f32, y: f32 },
    Scroll { request: u64, x: f32, y: f32 },
}

#[derive(Debug)]
pub struct Viewer {
    pub position: ReadingPosition,
    pub count: Option<u32>,
    pub page_input: String,
    pub image: Option<image::Handle>,
    pub image_ratio: f32,
    pub display_width: f32,
    pub loading: bool,
    pub error: String,
    pub request: u64,
    pub restoring: bool,
    pub keep_scroll: bool,
    pub hovered: bool,
    pub page_window: u32,
    pub pages: BTreeMap<u32, (image::Handle, f32)>,
}

impl Viewer {
    pub fn new(position: ReadingPosition) -> Self {
        Self {
            position,
            count: None,
            page_input: position.page.saturating_add(1).to_string(),
            image: None,
            image_ratio: 1.5,
            display_width: 600.0,
            loading: false,
            error: String::new(),
            request: 0,
            restoring: false,
            keep_scroll: false,
            hovered: false,
            page_window: 2,
            pages: BTreeMap::new(),
        }
    }

    pub fn scroll_id(id: BufferId) -> String {
        format!("document-scroll-{}", id.0)
    }

    pub fn cache_page(&mut self, page: &RenderedPage) {
        self.pages.insert(
            page.page,
            (
                image::Handle::from_rgba(page.width, page.height, page.rgba.to_vec()),
                page.height as f32 / page.width as f32,
            ),
        );
        self.pages.retain(|number, _| {
            *number >= self.position.page.saturating_sub(1)
                && *number < self.position.page.saturating_add(self.page_window)
        });
    }

    pub fn slot_height(&self) -> f32 {
        self.display_width * 1.5 + 12.0
    }

    pub fn view(&self, id: BufferId, enabled: bool) -> Element<'_, Message> {
        let tool = move |icon: Icon,
                         name: &'static str,
                         action: ReaderAction,
                         allowed: bool|
              -> Element<'_, Message> {
            tooltip(
                button(container(look::icon(icon)).center(24))
                    .padding(0)
                    .style(look::quiet)
                    .on_press_maybe((enabled && allowed).then_some(Message::Reader(id, action))),
                text(name).size(12),
                tooltip::Position::Top,
            )
            .into()
        };
        let footer: Element<'_, Message> = if self.hovered {
            scrollable(
                row![
                    tool(
                        Icon::ChevronLeft,
                        "Previous page",
                        ReaderAction::Previous,
                        self.position.page > 0
                    ),
                    text_input("Page", &self.page_input)
                        .size(11)
                        .padding(3)
                        .width(40)
                        .on_input(move |value| Message::Reader(id, ReaderAction::PageInput(value)))
                        .on_submit(Message::Reader(id, ReaderAction::Go)),
                    text(format!("/ {}", self.count.unwrap_or(0))).size(11),
                    tool(
                        Icon::ChevronRight,
                        "Next page",
                        ReaderAction::Next,
                        self.count
                            .is_some_and(|count| self.position.page + 1 < count)
                    ),
                    tool(
                        Icon::Minus,
                        "Zoom out",
                        ReaderAction::ZoomOut,
                        self.position.zoom > 0.5
                    ),
                    text(format!("{}%", (self.position.zoom * 100.0).round() as u32)).size(11),
                    tool(
                        Icon::Plus,
                        "Zoom in",
                        ReaderAction::ZoomIn,
                        self.position.zoom < 3.0
                    ),
                    tool(Icon::Maximize, "Fit width", ReaderAction::Fit, true),
                ]
                .spacing(4)
                .align_y(iced::Center),
            )
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .into()
        } else {
            Space::new().height(24).into()
        };
        let content: Element<'_, Message> = if self.image.is_none() {
            container(
                column![
                    text(if self.error.is_empty() {
                        "Loading..."
                    } else {
                        &self.error
                    })
                    .size(13),
                    tool(Icon::RefreshCw, "Retry", ReaderAction::Retry, !self.loading)
                ]
                .spacing(8),
            )
            .center(Fill)
            .into()
        } else {
            let request = self.request;
            let count = self.count.unwrap_or(1);
            let first = self.position.page.saturating_sub(1);
            let last = self
                .position
                .page
                .saturating_add(self.page_window)
                .min(count);
            let slot = self.slot_height();
            let mut pages = column![Space::new().height(first as f32 * slot)];
            for page in first..last {
                let content: Element<'_, Message> = if let Some((handle, _)) = self.pages.get(&page)
                {
                    image(handle.clone())
                        .width(self.display_width)
                        .height(slot - 12.0)
                        .content_fit(iced::ContentFit::Contain)
                        .into()
                } else {
                    Space::new()
                        .width(self.display_width)
                        .height(slot - 12.0)
                        .into()
                };
                pages = pages.push(container(content).height(slot).padding(iced::Padding {
                    bottom: 12.0,
                    ..Default::default()
                }));
            }
            pages = pages.push(Space::new().height((count - last) as f32 * slot));
            scrollable(container(pages).padding([0, 12]))
                .id(Self::scroll_id(id))
                .direction(scrollable::Direction::Both {
                    vertical: scrollable::Scrollbar::default(),
                    horizontal: scrollable::Scrollbar::default(),
                })
                .height(Fill)
                .width(Fill)
                .on_scroll(move |viewport| {
                    let offset = viewport.absolute_offset();
                    Message::Reader(
                        id,
                        ReaderAction::StreamScroll {
                            request,
                            x: offset.x,
                            y: offset.y,
                        },
                    )
                })
                .into()
        };
        mouse_area(
            column![content, container(footer).height(32).padding([4, 8])]
                .height(Fill)
                .width(Fill),
        )
        .on_enter(Message::Reader(id, ReaderAction::Hover(true)))
        .on_exit(Message::Reader(id, ReaderAction::Hover(false)))
        .into()
    }
}
