use crate::{app::Message, appearance as look};
use enote::{
    buffer::BufferId,
    reader::{ReadingPosition, RenderWorker, RenderedPage},
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
pub struct CachedPage {
    pub handle: image::Handle,
    pub rendered: RenderedPage,
    pub render_width: f32,
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
    pub pages: BTreeMap<u32, CachedPage>,
    pub worker: Option<RenderWorker>,
    pub pixel_width: f32,
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
            page_window: 4,
            pages: BTreeMap::new(),
            worker: None,
            pixel_width: 1200.0,
        }
    }

    pub fn scroll_id(id: BufferId) -> String {
        format!("document-scroll-{}", id.0)
    }

    pub fn cached_page(&self, page: u32) -> Option<&RenderedPage> {
        self.pages
            .get(&page)
            .filter(|cached| cached.render_width == self.pixel_width)
            .map(|cached| &cached.rendered)
    }

    pub fn cache_page(&mut self, page: &RenderedPage) {
        // Keep a stable image handle: recreating it forces another GPU upload.
        if self.cached_page(page.page).is_some() {
            return;
        }
        self.pages.insert(
            page.page,
            CachedPage {
                handle: image::Handle::from_rgba(
                    page.width,
                    page.height,
                    bytes::Bytes::from_owner(page.rgba.clone()),
                ),
                rendered: page.clone(),
                render_width: self.pixel_width,
            },
        );
        // Keep recently visited pages in both directions. Old-resolution pages
        // remain visible until their replacements arrive, with a bounded budget.
        let mut bytes: usize = self.pages.values().map(|p| p.rendered.rgba.len()).sum();
        while self.pages.len() > 16 || bytes > 128 * 1024 * 1024 {
            let Some(farthest) = self
                .pages
                .keys()
                .filter(|n| **n != self.position.page)
                .max_by_key(|n| (n.abs_diff(self.position.page), **n < self.position.page))
                .copied()
            else {
                break;
            };
            bytes -= self.pages.remove(&farthest).unwrap().rendered.rgba.len();
        }
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
            let first = self.position.page.saturating_sub(2);
            let last = self
                .position
                .page
                .saturating_add(self.page_window)
                .min(count);
            let slot = self.slot_height();
            let mut pages = column![Space::new().height(first as f32 * slot)];
            for page in first..last {
                let content: Element<'_, Message> = if let Some(cached) = self.pages.get(&page) {
                    image(cached.handle.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn page(number: u32) -> RenderedPage {
        RenderedPage {
            page: number,
            count: 100,
            width: 2,
            height: 3,
            rgba: vec![255; 24].into(),
        }
    }

    #[test]
    fn cache_reuses_texture_handles_and_keeps_old_resolution_visible() {
        let mut viewer = Viewer::new(Default::default());
        let rendered = page(0);
        viewer.cache_page(&rendered);
        let handle = viewer.pages[&0].handle.clone();
        let image::Handle::Rgba { pixels, .. } = &handle else {
            panic!("Expected RGBA image");
        };
        assert_eq!(pixels.as_ptr(), rendered.rgba.as_ptr());
        viewer.cache_page(&page(0));
        assert_eq!(viewer.pages[&0].handle, handle);
        viewer.pixel_width += 64.0;
        assert!(viewer.cached_page(0).is_none());
        assert_eq!(viewer.pages[&0].handle, handle);
        viewer.cache_page(&page(0));
        assert_ne!(viewer.pages[&0].handle, handle);
    }

    #[test]
    fn cache_is_bounded_and_retains_nearby_pages_for_reverse_scrolling() {
        let mut viewer = Viewer::new(Default::default());
        viewer.position.page = 5;
        for number in 0..40 {
            viewer.cache_page(&page(number));
        }
        assert!(viewer.pages.len() <= 16);
        assert!(viewer.pages.contains_key(&5));
        assert!(viewer.pages.contains_key(&4));
        assert!(viewer.pages.contains_key(&6));
    }
}
