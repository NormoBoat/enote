use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
    widget::{Operation, Tree},
};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse, touch};

#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Press,
    Move,
    Release,
    RightPress,
    Leave,
}

pub struct DragArea<'a, Message> {
    content: Element<'a, Message>,
    on_event: Option<Box<dyn Fn(Phase, Point) -> Message + 'a>>,
    interaction: mouse::Interaction,
}

#[derive(Default)]
struct State {
    hovered: bool,
    position: Option<Point>,
}

fn event_position(event: &Event, cursor: mouse::Cursor, previous: Option<Point>) -> Option<Point> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position })
        | Event::Touch(touch::Event::FingerPressed { position, .. })
        | Event::Touch(touch::Event::FingerMoved { position, .. })
        | Event::Touch(touch::Event::FingerLifted { position, .. }) => Some(*position),
        _ => previous.or_else(|| cursor.position()),
    }
}

impl<'a, Message> DragArea<'a, Message> {
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        interaction: mouse::Interaction,
        on_event: impl Fn(Phase, Point) -> Message + 'a,
    ) -> Self {
        Self {
            content: content.into(),
            interaction,
            on_event: Some(Box::new(on_event)),
        }
    }

    pub fn tracking(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            on_event: None,
            interaction: mouse::Interaction::None,
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for DragArea<'_, Message> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<State>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let position = event_position(event, cursor, tree.state.downcast_ref::<State>().position);
        if matches!(
            event,
            Event::Mouse(mouse::Event::CursorMoved { .. }) | Event::Touch(_)
        ) {
            tree.state.downcast_mut::<State>().position = position;
        }
        // iced may batch input with a cursor already at the final event position.
        let event_cursor = position.map(mouse::Cursor::Available).unwrap_or(cursor);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            event_cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let Some(on_event) = &self.on_event else {
            return;
        };
        let phase = match event {
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => Phase::Move,
            Event::Touch(touch::Event::FingerPressed { .. }) => Phase::Press,
            Event::Touch(touch::Event::FingerLifted { .. }) => Phase::Release,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Phase::Press,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Phase::Release,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => Phase::RightPress,
            _ => return,
        };
        let Some(position) = position else {
            return;
        };
        // Use event coordinates and clipped row bounds, not the last hover message.
        let over = layout
            .bounds()
            .intersection(viewport)
            .is_some_and(|bounds| bounds.contains(position));
        if matches!(phase, Phase::Move) {
            let state = tree.state.downcast_mut::<State>();
            if over || state.hovered {
                shell.publish(on_event(if over { phase } else { Phase::Leave }, position));
            }
            if over {
                shell.capture_event();
            }
            state.hovered = over;
        } else if over && !shell.is_event_captured() {
            tree.state.downcast_mut::<State>().hovered = true;
            shell.publish(on_event(phase, position));
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let child = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        if child != mouse::Interaction::None {
            child
        } else if cursor.position().is_some_and(|position| {
            layout
                .bounds()
                .intersection(viewport)
                .is_some_and(|bounds| bounds.contains(position))
        }) {
            self.interaction
        } else {
            mouse::Interaction::None
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<DragArea<'a, Message>> for Element<'a, Message> {
    fn from(area: DragArea<'a, Message>) -> Self {
        Element::new(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batched_press_and_release_keep_event_positions_not_final_cursor() {
        let start = Point::new(20.0, 30.0);
        let end = Point::new(20.0, 100.0);
        let cursor = mouse::Cursor::Available(end);
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: start });
        let previous = event_position(&moved, cursor, None);
        assert_eq!(
            event_position(
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                cursor,
                previous
            ),
            Some(start)
        );
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: end });
        let previous = event_position(&moved, cursor, previous);
        assert_eq!(
            event_position(
                &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                cursor,
                previous
            ),
            Some(end)
        );
    }
}
