use crate::{
    appearance as look,
    drag_area::{DragArea, Phase},
    explorer::{Directory, Explorer},
    file_manager::{
        ContextAction, ContextMenu, Drag, FileOperation, FileOutcome, FilePrompt, Target,
    },
    viewer::{ReaderAction, Viewer},
    volume_session::{OpenedVolume, Startup},
};
use enote::buffer::{BufferContent, BufferId, Buffers, LoadedFile, MediaKind, SavedFile};
use enote::{
    file_ops::TrashRecord,
    reader::{ReaderStore, RenderedPage},
    session::{PaneLayout, SessionStore, WindowPlacement, WorkspaceSession},
    settings::{LoadedSettings, Settings},
    shortcuts::{Modifiers as ShortcutModifiers, ShortcutAction},
    volume::EntryKind,
    volume::VolumeStorage,
};
use iced::{
    Element, Event, Fill, Subscription, Task, keyboard,
    widget::{
        Space, button, column, container, image, mouse_area, opaque, pane_grid, row, scrollable,
        stack, text, text_editor, text_input, tooltip,
    },
};
use lucide_icons::Icon;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
struct TabDrag {
    serial: u64,
    buffer: BufferId,
    start: iced::Point,
    moving: bool,
}

fn tab_drop_region(point: iced::Point, bounds: iced::Rectangle) -> pane_grid::Region {
    let distances = [
        (
            (point.x - bounds.x) / bounds.width.max(1.0),
            pane_grid::Edge::Left,
        ),
        (
            (bounds.x + bounds.width - point.x) / bounds.width.max(1.0),
            pane_grid::Edge::Right,
        ),
        (
            (point.y - bounds.y) / bounds.height.max(1.0),
            pane_grid::Edge::Top,
        ),
        (
            (bounds.y + bounds.height - point.y) / bounds.height.max(1.0),
            pane_grid::Edge::Bottom,
        ),
    ];
    let (distance, edge) = distances
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    if distance < 0.22 {
        pane_grid::Region::Edge(edge)
    } else {
        pane_grid::Region::Center
    }
}

#[derive(Debug, Clone)]
struct EntryDraft {
    parent: PathBuf,
    kind: EntryKind,
    name: String,
    source: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct CreatedEntry {
    parent: Directory,
    folder: Option<Directory>,
    file: Option<LoadedFile>,
}

impl CreatedEntry {
    fn create(volume: &VolumeStorage, draft: &EntryDraft) -> Result<Self, String> {
        let path = volume.create_entry(&draft.parent, &draft.name, draft.kind)?;
        Ok(Self {
            parent: Directory::read(&draft.parent)?,
            folder: if draft.kind == EntryKind::Folder {
                Some(Directory::read(&path)?)
            } else {
                None
            },
            file: if draft.kind == EntryKind::File {
                Some(LoadedFile::read(&path)?)
            } else {
                None
            },
        })
    }
}

pub struct Enote {
    settings: Settings,
    base_settings: LoadedSettings,
    session: Option<SessionStore>,
    window_position: Option<[f32; 2]>,
    session_pending: bool,
    session_revision: u64,
    session_io: Arc<Mutex<u64>>,
    volume: Option<VolumeStorage>,
    volume_loading: bool,
    creation_name: Option<String>,
    entry_draft: Option<EntryDraft>,
    hovered_folder: Option<PathBuf>,
    selected_folder: Option<PathBuf>,
    editor_menu: bool,
    context_menu: Option<ContextMenu>,
    file_prompt: Option<FilePrompt>,
    destination_loading: bool,
    hovered_target: Option<Target>,
    selected_entry: Option<PathBuf>,
    cursor: iced::Point,
    drag: Option<Drag>,
    last_deleted: Option<TrashRecord>,
    buffers: Buffers,
    editors: BTreeMap<BufferId, text_editor::Content>,
    images: BTreeMap<BufferId, image::Handle>,
    readers: BTreeMap<BufferId, Viewer>,
    reader_store: Option<ReaderStore>,
    reader_request: u64,
    reader_save_pending: bool,
    pending_imports: std::collections::VecDeque<FileOperation>,
    importing: bool,
    import_errors: Vec<String>,
    active: Option<BufferId>,
    tab_order: Vec<BufferId>,
    tab_drag: Option<TabDrag>,
    tab_drag_serial: u64,
    tab_hover: Option<(BufferId, bool)>,
    tab_pane_hover: Option<(pane_grid::Pane, pane_grid::Region)>,
    panes: pane_grid::State<Option<BufferId>>,
    focused_pane: pane_grid::Pane,
    resizing_sidebar: bool,
    explorer: Explorer,
    sidebar: bool,
    viewport_width: f32,
    viewport_height: f32,
    busy: bool,
    status: String,
    pending_close: Option<CloseTarget>,
}

#[derive(Debug, Clone, Copy)]
pub enum CloseTarget {
    Buffer(BufferId),
    Window,
    SwitchVolume(VolumeAction),
}

#[derive(Debug, Clone, Copy)]
pub enum VolumeAction {
    Open,
    Create,
}

#[derive(Debug, Clone)]
pub enum Message {
    Split(pane_grid::Axis),
    SplitPane(pane_grid::Pane, pane_grid::Axis),
    FocusPane(pane_grid::Pane),
    ClosePane(pane_grid::Pane),
    ResizePane(pane_grid::ResizeEvent),
    DragPane(pane_grid::DragEvent),
    TabPointer(BufferId, Phase, iced::Point, iced::Rectangle),
    PanePointer(pane_grid::Pane, Phase, iced::Point, iced::Rectangle),
    EndTabDrag(u64),
    WindowMoved(iced::Point),
    WindowOpened(Option<iced::Point>, iced::Size),
    RestoreReaders,
    PersistSession,
    SessionSaved(Result<(), String>),
    ResizeSidebar(Phase, iced::Point),
    ReaderPrefetched(BufferId, u64, Result<RenderedPage, String>),
    FileDropped(PathBuf),
    Reader(BufferId, ReaderAction),
    ReaderRendered(BufferId, u64, Result<RenderedPage, String>),
    ReaderRestored(BufferId, u64),
    PersistReaders,
    ReadersPersisted(uuid::Uuid, Result<(), String>),
    RootDrop(Phase, iced::Point),
    PointerEntry(Target, Phase, iced::Point),
    ContextRequested(PathBuf, EntryKind),
    ContextAction(ContextAction),
    DismissContext,
    HoverEntry(PathBuf, EntryKind),
    ExitEntry(PathBuf),
    ArmEntry(PathBuf, EntryKind),
    MouseUp,
    Viewport(iced::Size),
    DestinationRequested(PathBuf),
    DestinationLoaded(PathBuf, Result<Directory, String>),
    ConfirmFilePrompt,
    CancelFilePrompt,
    FileCompleted(Result<FileOutcome, String>),
    New,
    NewFolder,
    HoverFolder(PathBuf, bool),
    CursorMoved(iced::Point),
    ClearHover,
    BeginEntry(PathBuf, EntryKind),
    EntryName(String),
    SubmitEntry,
    CancelEntry,
    EntryCreated(Result<CreatedEntry, String>),
    KeyPressed(String, keyboard::Modifiers),
    SaveAs,
    CloseActiveTab,
    NextTab,
    PreviousTab,
    ToggleEditorMenu,
    Open,
    OpenVolume,
    CreateVolume,
    CreateName(String),
    ConfirmCreate,
    CancelCreate,
    VolumeOpened(Option<Result<OpenedVolume, String>>),
    Remembered(Result<(), String>),
    OpenFile(PathBuf),
    ToggleFolder(PathBuf),
    DirectoryLoaded(PathBuf, Result<Directory, String>),
    ToggleSidebar,
    Opened(Option<Result<LoadedFile, String>>),
    Activate(BufferId),
    Edit(BufferId, text_editor::Action),
    Save,
    Undo,
    Redo,
    SavePath(BufferId, Result<Option<PathBuf>, String>),
    Saved(Result<SavedFile, String>),
    Close(CloseTarget),
    Discard,
    Cancel,
}

impl Default for Enote {
    fn default() -> Self {
        Self::with_settings(Settings::default(), String::new())
    }
}

impl Enote {
    pub fn with_settings(settings: Settings, warnings: String) -> Self {
        let (panes, focused_pane) = pane_grid::State::new(None);
        Self {
            panes,
            focused_pane,
            resizing_sidebar: false,
            base_settings: LoadedSettings::from_settings(settings.clone()),
            session: None,
            window_position: None,
            session_pending: false,
            session_revision: 0,
            session_io: Arc::new(Mutex::new(0)),
            volume: None,
            volume_loading: false,
            creation_name: None,
            entry_draft: None,
            hovered_folder: None,
            selected_folder: None,
            editor_menu: false,
            context_menu: None,
            file_prompt: None,
            destination_loading: false,
            hovered_target: None,
            selected_entry: None,
            cursor: iced::Point::ORIGIN,
            drag: None,
            last_deleted: None,
            buffers: Buffers::default(),
            editors: BTreeMap::new(),
            images: BTreeMap::new(),
            readers: BTreeMap::new(),
            reader_store: None,
            reader_request: 0,
            reader_save_pending: false,
            pending_imports: Default::default(),
            importing: false,
            import_errors: Vec::new(),
            active: None,
            tab_order: Vec::new(),
            tab_drag: None,
            tab_drag_serial: 0,
            tab_hover: None,
            tab_pane_hover: None,
            explorer: Explorer::default(),
            sidebar: settings.workspace.sidebar_visible,
            viewport_width: settings.window.width as f32,
            viewport_height: settings.window.height as f32,
            busy: false,
            status: warnings,
            pending_close: None,
            settings,
        }
    }

    pub fn from_startup(startup: Startup) -> Self {
        let mut app = Self::with_settings(startup.base.settings.clone(), startup.warnings.clone());
        app.base_settings = startup.base;
        app.session = startup.session;
        if let Some(volume) = startup.volume
            && let Err(error) = app.apply_volume(volume)
        {
            app.status = error;
        }
        if !startup.warnings.is_empty() {
            app.status = startup.warnings;
        }
        if let Some(window) = startup.window {
            app.viewport_width = window.width;
            app.viewport_height = window.height;
            app.window_position = window.position;
        } else {
            app.viewport_width = app.settings.window.width as f32;
            app.viewport_height = app.settings.window.height as f32;
        }
        app
    }
    pub fn startup_tasks(&mut self) -> Task<Message> {
        Task::done(Message::RestoreReaders)
    }
    pub fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, _, _| match event {
            Event::Window(iced::window::Event::CloseRequested) => {
                Some(Message::Close(CloseTarget::Window))
            }
            Event::Window(iced::window::Event::Opened { position, size }) => {
                Some(Message::WindowOpened(position, size))
            }
            Event::Window(iced::window::Event::Moved(point)) => Some(Message::WindowMoved(point)),
            Event::Window(iced::window::Event::Resized(size)) => Some(Message::Viewport(size)),
            Event::Window(iced::window::Event::FileDropped(path)) => {
                Some(Message::FileDropped(path))
            }
            Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => {
                Some(Message::MouseUp)
            }
            Event::Touch(iced::touch::Event::FingerLifted { .. }) => Some(Message::MouseUp),
            Event::Touch(iced::touch::Event::FingerLost { .. }) => Some(Message::ClearHover),
            Event::Touch(iced::touch::Event::FingerMoved { position, .. }) => {
                Some(Message::CursorMoved(position))
            }
            Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Message::CursorMoved(position))
            }
            Event::Mouse(iced::mouse::Event::CursorLeft)
            | Event::Window(iced::window::Event::Unfocused) => Some(Message::ClearHover),
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                modifiers,
                ..
            }) => Some(Message::KeyPressed(
                Self::key_name(&key, physical_key),
                modifiers,
            )),
            _ => None,
        })
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        self.sync_tab_order();
        let persist = matches!(
            &message,
            Message::Opened(_)
                | Message::Activate(_)
                | Message::Close(_)
                | Message::Discard
                | Message::Saved(_)
                | Message::FileCompleted(_)
                | Message::EntryCreated(_)
                | Message::Split(_)
                | Message::SplitPane(_, _)
                | Message::ClosePane(_)
                | Message::FocusPane(_)
                | Message::ResizePane(_)
                | Message::DragPane(_)
                | Message::TabPointer(_, Phase::Release, _, _)
                | Message::PanePointer(_, Phase::Release, _, _)
                | Message::NextTab
                | Message::PreviousTab
                | Message::Viewport(_)
                | Message::WindowMoved(_)
                | Message::WindowOpened(_, _)
        );
        let task = self.update_inner(message);
        self.sync_tab_order();
        if self
            .tab_drag
            .as_ref()
            .is_some_and(|drag| !self.buffers.entries.contains_key(&drag.buffer))
        {
            self.clear_tab_drag();
        }
        for (_, buffer) in self.panes.iter_mut() {
            if buffer.is_some_and(|id| !self.buffers.entries.contains_key(&id)) {
                *buffer = None;
            }
        }
        if let Some(id) = self.active {
            let existing = self
                .panes
                .iter()
                .find_map(|(pane, value)| (*value == Some(id)).then_some(*pane));
            if let Some(pane) = existing {
                self.focused_pane = pane;
            }
        }
        if let Some(value) = self.panes.get_mut(self.focused_pane) {
            *value = self.active;
        }
        if persist {
            Task::batch([task, self.schedule_session_save()])
        } else {
            task
        }
    }

    fn update_inner(&mut self, message: Message) -> Task<Message> {
        if self.file_prompt.is_some()
            && !matches!(
                message,
                Message::DestinationRequested(_)
                    | Message::DestinationLoaded(_, _)
                    | Message::DirectoryLoaded(_, _)
                    | Message::ReaderRendered(_, _, _)
                    | Message::ReaderRestored(_, _)
                    | Message::PersistReaders
                    | Message::ReadersPersisted(_, _)
                    | Message::ReaderPrefetched(_, _, _)
                    | Message::ConfirmFilePrompt
                    | Message::CancelFilePrompt
                    | Message::KeyPressed(_, _)
                    | Message::Viewport(_)
                    | Message::WindowMoved(_)
                    | Message::WindowOpened(_, _)
                    | Message::PersistSession
                    | Message::SessionSaved(_)
                    | Message::RestoreReaders
                    | Message::CursorMoved(_)
                    | Message::ClearHover
                    | Message::Close(CloseTarget::Window)
            )
        {
            return Task::none();
        }
        match message {
            Message::TabPointer(id, phase, point, bounds)
                if !self.busy && self.pending_close.is_none() && !self.volume_loading =>
            {
                match phase {
                    Phase::Press => {
                        self.clear_tab_drag();
                        self.drag = None;
                        self.tab_drag_serial += 1;
                        self.tab_drag = Some(TabDrag {
                            serial: self.tab_drag_serial,
                            buffer: id,
                            start: point,
                            moving: false,
                        });
                    }
                    Phase::Move => {
                        if let Some(drag) = &mut self.tab_drag {
                            drag.moving |= drag.start.distance(point) > 5.0;
                            self.tab_hover = Some((id, point.x < bounds.center_x()));
                            self.tab_pane_hover = None;
                        }
                    }
                    Phase::Leave => {
                        if self.tab_hover.is_some_and(|(target, _)| target == id) {
                            self.tab_hover = None;
                        }
                    }
                    Phase::Release => {
                        if let Some(drag) = self.tab_drag.take() {
                            let moving = drag.moving || drag.start.distance(point) > 5.0;
                            self.clear_tab_drag();
                            if moving {
                                self.reorder_tab(drag.buffer, id, point.x < bounds.center_x());
                            } else if drag.buffer == id {
                                return self.update(Message::Activate(id));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Message::PanePointer(pane, phase, point, bounds) => {
                if let Some(drag) = &mut self.tab_drag {
                    drag.moving |= drag.start.distance(point) > 5.0;
                    match phase {
                        Phase::Move if drag.moving => {
                            self.tab_hover = None;
                            self.tab_pane_hover = Some((pane, tab_drop_region(point, bounds)));
                        }
                        Phase::Leave => {
                            if self
                                .tab_pane_hover
                                .is_some_and(|(target, _)| target == pane)
                            {
                                self.tab_pane_hover = None;
                            }
                        }
                        Phase::Release => {
                            let buffer = drag.buffer;
                            let moving = drag.moving;
                            self.clear_tab_drag();
                            if moving && self.pending_close.is_none() && !self.busy {
                                return self.drop_tab(buffer, pane, tab_drop_region(point, bounds));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Message::PersistSession => {
                self.session_pending = false;
                return self.flush_session();
            }
            Message::SessionSaved(Err(error)) => self.status = error,
            Message::WindowMoved(point) => {
                if point.x.is_finite() && point.y.is_finite() {
                    self.window_position = Some([point.x, point.y]);
                }
            }
            Message::WindowOpened(position, size) => {
                if let Some(point) = position {
                    self.window_position = Some([point.x, point.y]);
                }
                return self.update_inner(Message::Viewport(size));
            }
            Message::RestoreReaders => {
                let ids: Vec<_> = self
                    .panes
                    .iter()
                    .filter_map(|(_, id)| *id)
                    .filter(|id| self.readers.contains_key(id))
                    .collect();
                return Task::batch(ids.into_iter().map(|id| self.render_reader(id)));
            }
            Message::DragPane(event)
                if self.pending_close.is_none()
                    && !self.volume_loading
                    && self.file_prompt.is_none() =>
            {
                match event {
                    pane_grid::DragEvent::Picked { pane } => {
                        if let Some(value) = self.panes.get(pane) {
                            self.focused_pane = pane;
                            self.active = *value;
                        }
                    }
                    pane_grid::DragEvent::Dropped { pane, target } => {
                        let valid_target = match target {
                            pane_grid::Target::Pane(target, _) => {
                                target != pane && self.panes.get(target).is_some()
                            }
                            pane_grid::Target::Edge(_) => true,
                        };
                        if self.panes.get(pane).is_some() && valid_target {
                            let dragged = *self.panes.get(pane).unwrap();
                            self.panes.drop(pane, target);
                            self.active = dragged;
                            self.focused_pane = self
                                .panes
                                .iter()
                                .find_map(|(pane, value)| (*value == dragged).then_some(*pane))
                                .unwrap_or(pane);
                            return self.resize_readers();
                        }
                    }
                    pane_grid::DragEvent::Canceled { .. } => {}
                }
            }
            Message::SplitPane(pane, axis) => {
                if let Some(value) = self.panes.get(pane) {
                    self.focused_pane = pane;
                    self.active = *value;
                }
                return self.update_inner(Message::Split(axis));
            }
            Message::Split(axis) if self.pending_close.is_none() => {
                let next = self
                    .buffers
                    .entries
                    .keys()
                    .find(|id| !self.panes.iter().any(|(_, value)| *value == Some(**id)))
                    .copied();
                if let Some((pane, _)) = self.panes.split(axis, self.focused_pane, next) {
                    self.focused_pane = pane;
                    self.active = next;
                }
                return self.resize_readers();
            }
            Message::FocusPane(pane) => {
                if let Some(value) = self.panes.get(pane) {
                    self.focused_pane = pane;
                    self.active = *value;
                }
            }
            Message::ClosePane(pane) if self.pending_close.is_none() => {
                if let Some((_, sibling)) = self.panes.close(pane) {
                    self.focused_pane = sibling;
                    self.active = *self.panes.get(sibling).unwrap();
                }
                return self.resize_readers();
            }
            Message::ResizePane(event) => {
                self.panes.resize(event.split, event.ratio.clamp(0.1, 0.9));
                return self.resize_readers();
            }
            Message::ResizeSidebar(phase, position) => {
                if matches!(phase, Phase::Press) {
                    self.resizing_sidebar = true;
                }
                if self.resizing_sidebar {
                    self.settings.workspace.sidebar_width =
                        (self.viewport_width - position.x - 38.0)
                            .clamp(180.0, (self.viewport_width * 0.6).max(180.0))
                            as u16;
                    if matches!(phase, Phase::Release) {
                        self.resizing_sidebar = false;
                    }
                    return self.resize_readers();
                }
            }
            Message::ReaderPrefetched(id, request, Ok(page)) => {
                if let Some(viewer) = self.readers.get_mut(&id)
                    && viewer.request == request
                {
                    viewer.cache_page(&page);
                }
            }
            Message::Reader(id, action)
                if self.pending_close.is_none()
                    && self.file_prompt.is_none()
                    && !self.volume_loading =>
            {
                return self.reader_action(id, action);
            }
            Message::ReaderRendered(id, request, result) => {
                let available = self.reader_width_for(id);
                if let Some(viewer) = self.readers.get_mut(&id)
                    && viewer.request == request
                {
                    viewer.loading = false;
                    match result {
                        Ok(page) => {
                            viewer.cache_page(&page);
                            if viewer.position.page != page.page && viewer.count.is_some() {
                                return self.render_reader(id);
                            }
                            viewer.position.page = page.page;
                            viewer.count = Some(page.count);
                            viewer.page_input = (page.page + 1).to_string();
                            viewer.image_ratio = page.height as f32 / page.width as f32;
                            viewer.image = Some(viewer.pages[&page.page].handle.clone());
                            viewer.error.clear();
                            if (viewer.display_width - available * viewer.position.zoom).abs() > 2.0
                            {
                                return self.render_reader(id);
                            }
                            let save = self.remember_reader(id);
                            if self.readers[&id].restoring
                                && (self.panes.iter().any(|(_, value)| *value == Some(id))
                                    || self.active == Some(id))
                            {
                                return Task::batch([save, self.restore_reader(id)]);
                            }
                            return save;
                        }
                        Err(error) => {
                            viewer.error = error;
                            viewer.restoring = false;
                        }
                    }
                }
            }
            Message::ReaderRestored(id, request) => {
                if let Some(viewer) = self.readers.get_mut(&id)
                    && viewer.request == request
                {
                    viewer.restoring = false;
                }
            }
            Message::PersistReaders => {
                self.reader_save_pending = false;
                return self.flush_readers();
            }
            Message::ReadersPersisted(volume, result) => {
                if self
                    .volume
                    .as_ref()
                    .is_some_and(|current| current.manifest.id == volume)
                    && let Err(error) = result
                {
                    self.status = error;
                }
            }
            Message::RootDrop(phase, position) => {
                if let Some(root) = self.explorer.root.clone() {
                    match phase {
                        Phase::Move | Phase::Release if self.drag.is_some() => {
                            return self.update(Message::PointerEntry(
                                Target {
                                    path: root,
                                    kind: EntryKind::Folder,
                                },
                                phase,
                                position,
                            ));
                        }
                        Phase::Leave => return self.update(Message::ExitEntry(root)),
                        _ => {}
                    }
                }
            }
            Message::PointerEntry(target, phase, position) => match phase {
                Phase::Leave => return self.update(Message::ExitEntry(target.path)),
                Phase::Press => {
                    self.cursor = position;
                    return self.update(Message::ArmEntry(target.path, target.kind));
                }
                Phase::Move => {
                    self.cursor = position;
                    if let Some(drag) = &mut self.drag
                        && drag.start.distance(position) > 5.0
                    {
                        drag.moving = true;
                    }
                    return self.update(if target.kind == EntryKind::Folder {
                        Message::HoverFolder(target.path, true)
                    } else {
                        Message::HoverEntry(target.path, target.kind)
                    });
                }
                Phase::RightPress => {
                    self.cursor = position;
                    return self.update(Message::ContextRequested(target.path, target.kind));
                }
                Phase::Release => {
                    if let Some(drag) = self.drag.take()
                        && !self.busy
                    {
                        if drag.moving || drag.start.distance(position) > 5.0 {
                            if self
                                .explorer
                                .root
                                .as_ref()
                                .is_some_and(|root| drag.accepts(&target, root))
                            {
                                return self.run_file_operation(FileOperation::Move {
                                    source: drag.source.path,
                                    destination: target.path,
                                });
                            }
                        } else if target.path == drag.source.path {
                            return self.update(if target.kind == EntryKind::Folder {
                                Message::ToggleFolder(target.path)
                            } else {
                                Message::OpenFile(target.path)
                            });
                        }
                    }
                }
            },
            Message::Viewport(size) => {
                self.viewport_width = size.width;
                self.viewport_height = size.height;
                return self.resize_readers();
            }
            Message::ClearHover => {
                self.resizing_sidebar = false;
                self.hovered_folder = None;
                self.hovered_target = None;
                self.drag = None;
                self.clear_tab_drag();
            }
            Message::CursorMoved(position) => {
                self.cursor = position;
                if let Some(drag) = &mut self.tab_drag {
                    drag.moving |= drag.start.distance(position) > 5.0;
                }
                if self.resizing_sidebar {
                    return self.update_inner(Message::ResizeSidebar(Phase::Move, position));
                }
                if let Some(drag) = &mut self.drag
                    && drag.start.distance(position) > 5.0
                {
                    drag.moving = true;
                }
                let panel_start = if self.viewport_width < 600.0 {
                    0.0
                } else {
                    self.viewport_width - f32::from(self.settings.workspace.sidebar_width) - 38.0
                };
                if !self.sidebar
                    || position.x < panel_start
                    || position.x >= self.viewport_width - 38.0
                {
                    self.hovered_folder = None;
                    self.hovered_target = None;
                }
            }
            Message::HoverEntry(path, kind) => {
                self.hovered_target = Some(Target { path, kind });
            }
            Message::ExitEntry(path) => {
                if self
                    .hovered_target
                    .as_ref()
                    .is_some_and(|target| target.path == path)
                {
                    self.hovered_target = None;
                }
                if self.hovered_folder.as_ref() == Some(&path) {
                    self.hovered_folder = None;
                }
            }
            Message::ArmEntry(path, kind)
                if !self.busy
                    && self.context_menu.is_none()
                    && self.file_prompt.is_none()
                    && self.entry_draft.is_none()
                    && self.pending_close.is_none() =>
            {
                self.selected_entry = Some(path.clone());
                self.drag = Some(Drag {
                    source: Target { path, kind },
                    start: self.cursor,
                    moving: false,
                });
            }
            Message::MouseUp => {
                self.drag = None;
                self.resizing_sidebar = false;
                // Let widget release messages from this input batch resolve the
                // drop before clearing a drag released outside any target.
                if let Some(drag) = &self.tab_drag {
                    return Task::done(Message::EndTabDrag(drag.serial));
                }
            }
            Message::EndTabDrag(serial) => {
                if self
                    .tab_drag
                    .as_ref()
                    .is_some_and(|drag| drag.serial == serial)
                {
                    self.clear_tab_drag();
                }
            }
            Message::ContextRequested(path, kind) if !self.busy && self.pending_close.is_none() => {
                self.entry_draft = None;
                self.drag = None;
                self.selected_entry = Some(path.clone());
                self.selected_folder = if kind == EntryKind::Folder {
                    Some(path.clone())
                } else {
                    path.parent().map(Path::to_owned)
                };
                self.context_menu = Some(ContextMenu {
                    target: Target { path, kind },
                    position: self.cursor,
                });
            }
            Message::DismissContext => self.context_menu = None,
            Message::ContextAction(action) if !self.busy => {
                if let Some(menu) = self.context_menu.take() {
                    return self.context_action(menu.target, action);
                }
            }
            Message::DestinationRequested(path) if !self.busy => {
                if let Some(FilePrompt::Move { destination, .. }) = &mut self.file_prompt {
                    *destination = path.clone();
                    self.destination_loading = true;
                    let volume = self.volume.clone().unwrap();
                    let requested = path.clone();
                    return Task::perform(
                        async move {
                            volume
                                .directory_path(&requested)
                                .and_then(|path| Directory::read(&path))
                        },
                        move |result| Message::DestinationLoaded(path.clone(), result),
                    );
                }
            }
            Message::DestinationLoaded(path, result) => {
                if let Some(FilePrompt::Move {
                    destination,
                    directory,
                    ..
                }) = &mut self.file_prompt
                    && *destination == path
                {
                    self.destination_loading = false;
                    match result {
                        Ok(loaded) => {
                            *destination = loaded.path.clone();
                            *directory = loaded;
                            self.status.clear();
                        }
                        Err(error) => {
                            *destination = directory.path.clone();
                            self.status = error;
                        }
                    }
                }
            }
            Message::CancelFilePrompt if !self.busy => {
                self.file_prompt = None;
                self.destination_loading = false;
            }
            Message::ConfirmFilePrompt if !self.busy && !self.destination_loading => {
                if let Some(prompt) = self.file_prompt.take() {
                    return self.run_file_operation(match prompt {
                        FilePrompt::Move {
                            source,
                            destination,
                            ..
                        } => FileOperation::Move {
                            source,
                            destination,
                        },
                        FilePrompt::Delete { source, .. } => FileOperation::Trash(source),
                    });
                }
            }
            Message::FileDropped(source)
                if self.pending_close.is_none() && self.entry_draft.is_none() =>
            {
                let Some(volume) = &self.volume else {
                    self.status = "Open a volume before importing files".into();
                    return Task::none();
                };
                if self.busy && !self.importing {
                    self.status = "Wait for the current operation before importing files".into();
                    return Task::none();
                }
                let destination = self
                    .hovered_target
                    .as_ref()
                    .map(|target| {
                        if target.kind == EntryKind::Folder {
                            target.path.clone()
                        } else {
                            target.path.parent().unwrap_or(&volume.root).to_owned()
                        }
                    })
                    .unwrap_or_else(|| volume.root.clone());
                self.pending_imports.push_back(FileOperation::Import {
                    source,
                    destination,
                });
                if !self.importing {
                    self.importing = true;
                    self.import_errors.clear();
                    let operation = self.pending_imports.pop_front().unwrap();
                    return self.run_file_operation(operation);
                }
            }
            Message::FileCompleted(result) => {
                self.busy = false;
                self.drag = None;
                match result {
                    Ok(outcome) => {
                        self.apply_file_outcome(outcome);
                    }
                    Err(error) => {
                        if self.importing {
                            self.import_errors.push(error.clone());
                        }
                        self.status = error;
                    }
                }
                if self.importing {
                    if let Some(operation) = self.pending_imports.pop_front() {
                        return self.run_file_operation(operation);
                    }
                    self.importing = false;
                    if !self.import_errors.is_empty() {
                        self.status = self.import_errors.join("; ");
                    }
                }
                return self.flush_readers();
            }
            Message::KeyPressed(key, modifiers) => {
                if let Some(action) = self.shortcut(&key, modifiers) {
                    if !matches!(action, ShortcutAction::Cancel) {
                        self.context_menu = None;
                    }
                    let message = match action {
                        ShortcutAction::NewFile => Message::New,
                        ShortcutAction::NewFolder => Message::NewFolder,
                        ShortcutAction::Open => Message::Open,
                        ShortcutAction::Save => Message::Save,
                        ShortcutAction::SaveAs => Message::SaveAs,
                        ShortcutAction::ReaderNextPage => {
                            Message::Reader(self.active.unwrap(), ReaderAction::Next)
                        }
                        ShortcutAction::ReaderPreviousPage => {
                            Message::Reader(self.active.unwrap(), ReaderAction::Previous)
                        }
                        ShortcutAction::CloseTab => Message::CloseActiveTab,
                        ShortcutAction::Undo => Message::Undo,
                        ShortcutAction::Redo => Message::Redo,
                        ShortcutAction::NextTab => Message::NextTab,
                        ShortcutAction::PreviousTab => Message::PreviousTab,
                        ShortcutAction::ToggleSidebar => Message::ToggleSidebar,
                        ShortcutAction::Cancel if self.tab_drag.is_some() => {
                            Message::EndTabDrag(self.tab_drag.as_ref().unwrap().serial)
                        }
                        ShortcutAction::Cancel if self.drag.is_some() => Message::MouseUp,
                        ShortcutAction::Cancel if self.context_menu.is_some() => {
                            Message::DismissContext
                        }
                        ShortcutAction::Cancel if self.file_prompt.is_some() => {
                            Message::CancelFilePrompt
                        }
                        ShortcutAction::Cancel if self.pending_close.is_some() => Message::Cancel,
                        ShortcutAction::Cancel if self.entry_draft.is_some() => {
                            Message::CancelEntry
                        }
                        ShortcutAction::Cancel if self.creation_name.is_some() => {
                            Message::CancelCreate
                        }
                        ShortcutAction::Cancel => {
                            self.editor_menu = false;
                            return Task::none();
                        }
                    };
                    return self.update(message);
                }
            }
            Message::HoverFolder(path, entered) => {
                if entered {
                    self.hovered_target = Some(Target {
                        path: path.clone(),
                        kind: EntryKind::Folder,
                    });
                    self.hovered_folder = Some(path);
                } else if self.hovered_folder.as_ref() == Some(&path) {
                    self.hovered_folder = None;
                    if self
                        .hovered_target
                        .as_ref()
                        .is_some_and(|target| target.path == path)
                    {
                        self.hovered_target = None;
                    }
                }
            }
            Message::BeginEntry(parent, kind) if !self.busy && self.pending_close.is_none() => {
                return self.begin_entry(parent, kind);
            }
            Message::EntryName(name) if !self.busy => {
                if let Some(draft) = &mut self.entry_draft {
                    draft.name = name;
                }
            }
            Message::CancelEntry if !self.busy => {
                self.entry_draft = None;
                self.status.clear();
            }
            Message::SubmitEntry if !self.busy && self.entry_draft.is_some() => {
                let draft = self.entry_draft.clone().unwrap();
                if let Err(error) = VolumeStorage::validate_name(&draft.name) {
                    self.status = error;
                    return Task::none();
                }
                let Some(volume) = self.volume.clone() else {
                    return Task::none();
                };
                if let Some(source) = draft.source {
                    return self.run_file_operation(FileOperation::Rename {
                        source,
                        name: draft.name,
                    });
                }
                self.busy = true;
                return Task::perform(
                    async move { CreatedEntry::create(&volume, &draft) },
                    Message::EntryCreated,
                );
            }
            Message::EntryCreated(result) => {
                self.busy = false;
                match result {
                    Ok(created) => {
                        self.entry_draft = None;
                        self.status.clear();
                        self.explorer.loading.remove(&created.parent.path);
                        self.explorer.expanded.insert(created.parent.path.clone());
                        self.explorer
                            .directories
                            .insert(created.parent.path, created.parent.entries);
                        if let Some(folder) = created.folder {
                            self.selected_folder = Some(folder.path.clone());
                            self.explorer.expanded.insert(folder.path.clone());
                            self.explorer
                                .directories
                                .insert(folder.path, folder.entries);
                        }
                        if let Some(file) = created.file {
                            return self.update(Message::Opened(Some(Ok(file))));
                        }
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::ToggleEditorMenu => self.editor_menu = !self.editor_menu,
            Message::NextTab | Message::PreviousTab
                if self.pending_close.is_none() && !self.volume_loading =>
            {
                let ids = self.tab_order.clone();
                if !ids.is_empty() {
                    let current = self
                        .active
                        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
                        .unwrap_or(0);
                    let index = if matches!(message, Message::NextTab) {
                        (current + 1) % ids.len()
                    } else {
                        (current + ids.len() - 1) % ids.len()
                    };
                    return self.update(Message::Activate(ids[index]));
                }
            }
            Message::CloseActiveTab => {
                self.editor_menu = false;
                if let Some(id) = self.active {
                    return self.update(Message::Close(CloseTarget::Buffer(id)));
                }
            }
            Message::ToggleSidebar if self.pending_close.is_none() => {
                self.sidebar = !self.sidebar;
                return self.resize_readers();
            }
            Message::OpenVolume | Message::CreateVolume
                if !self.busy && self.pending_close.is_none() =>
            {
                let action = if matches!(message, Message::OpenVolume) {
                    VolumeAction::Open
                } else {
                    VolumeAction::Create
                };
                if self
                    .buffers
                    .entries
                    .values()
                    .any(|buffer| buffer.modified())
                {
                    self.pending_close = Some(CloseTarget::SwitchVolume(action));
                } else {
                    return self.flush_session().chain(self.begin_volume(action));
                }
            }
            Message::CreateName(name) if self.creation_name.is_some() && !self.busy => {
                self.creation_name = Some(name)
            }
            Message::CancelCreate if !self.busy => self.creation_name = None,
            Message::ConfirmCreate if !self.busy && self.creation_name.is_some() => {
                let name = self.creation_name.clone().unwrap();
                if let Err(error) = VolumeStorage::validate_name(&name) {
                    self.status = error;
                    return Task::none();
                }
                self.busy = true;
                self.volume_loading = true;
                let base = self.base_settings.clone();
                return Task::perform(
                    async move {
                        match crate::file_dialog::folder().await {
                            Ok(Some(parent)) => Some(OpenedVolume::create(&parent, &name, &base)),
                            Ok(None) => None,
                            Err(error) => Some(Err(error)),
                        }
                    },
                    Message::VolumeOpened,
                );
            }
            Message::VolumeOpened(result) => {
                self.busy = false;
                self.volume_loading = false;
                if let Some(result) = result {
                    match result.and_then(|volume| self.apply_volume(volume)) {
                        Ok(()) => {
                            let restore = Task::done(Message::RestoreReaders);
                            if let Some(session) = self.session.clone() {
                                self.busy = true;
                                self.volume_loading = true;
                                let root = self.volume.as_ref().unwrap().root.clone();
                                return Task::batch([
                                    restore,
                                    Task::perform(
                                        async move { session.remember(&root) },
                                        Message::Remembered,
                                    ),
                                ]);
                            }
                            return restore;
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            Message::Remembered(result) => {
                self.busy = false;
                self.volume_loading = false;
                if let Err(error) = result {
                    self.status = format!("Volume opened, but its location was not saved: {error}");
                }
            }
            Message::ToggleFolder(path) if self.pending_close.is_none() => {
                self.selected_entry = Some(path.clone());
                self.selected_folder = Some(path.clone());
                if self.explorer.expanded.remove(&path) {
                    return Task::none();
                }
                self.explorer.expanded.insert(path.clone());
                if !self.explorer.directories.contains_key(&path)
                    && self.explorer.loading.insert(path.clone())
                {
                    let target = path.clone();
                    return Task::perform(async move { Directory::read(&target) }, move |result| {
                        Message::DirectoryLoaded(path.clone(), result)
                    });
                }
            }
            Message::DirectoryLoaded(path, result) => {
                if self.explorer.loading.remove(&path) {
                    match result {
                        Ok(directory) => {
                            self.explorer.directories.insert(path, directory.entries);
                        }
                        Err(error) => {
                            self.explorer.expanded.remove(&path);
                            self.status = error;
                        }
                    }
                }
            }
            Message::OpenFile(path) if !self.busy && self.pending_close.is_none() => {
                self.selected_entry = Some(path.clone());
                self.busy = true;
                return Task::perform(
                    async move { Some(LoadedFile::read(&path)) },
                    Message::Opened,
                );
            }
            Message::New | Message::NewFolder
                if !self.busy
                    && self.pending_close.is_none()
                    && self.volume.is_some()
                    && self.creation_name.is_none() =>
            {
                let parent = self
                    .selected_folder
                    .clone()
                    .or_else(|| self.explorer.root.clone())
                    .unwrap();
                let kind = if matches!(message, Message::NewFolder) {
                    EntryKind::Folder
                } else {
                    EntryKind::File
                };
                return self.begin_entry(parent, kind);
            }
            Message::Open
                if !self.busy && self.pending_close.is_none() && self.creation_name.is_none() =>
            {
                if self.volume.is_none() {
                    return self.begin_volume(VolumeAction::Open);
                }
                self.busy = true;
                self.status = "Opening...".into();
                return Task::perform(
                    async {
                        match crate::file_dialog::open().await {
                            Ok(Some(path)) => Some(LoadedFile::read(&path)),
                            Ok(None) => None,
                            Err(error) => Some(Err(error)),
                        }
                    },
                    Message::Opened,
                );
            }
            Message::Opened(result) => {
                self.busy = false;
                self.status.clear();
                if let Some(result) = result {
                    match result.and_then(|loaded| {
                        if let Some(volume) = &self.volume {
                            volume.document_path(loaded.path())?;
                        }
                        self.selected_folder = loaded.path().parent().map(Path::to_owned);
                        self.selected_entry = Some(loaded.path().to_owned());
                        self.buffers.insert(loaded)
                    }) {
                        Ok(id) => {
                            let buffer = &self.buffers.entries[&id];
                            if let Some(value) = buffer.text() {
                                self.editors
                                    .entry(id)
                                    .or_insert_with(|| text_editor::Content::with_text(value));
                            }
                            if let BufferContent::Media {
                                kind: MediaKind::Image,
                                bytes: Some(bytes),
                                ..
                            } = &buffer.content
                            {
                                self.images
                                    .entry(id)
                                    .or_insert_with(|| image::Handle::from_bytes(bytes.to_vec()));
                            }
                            self.active = Some(id);
                            if self.viewport_width < 600.0 {
                                self.sidebar = false;
                            }
                            if buffer.text().is_some() {
                                return iced::widget::operation::focus("active-editor");
                            }
                            if matches!(
                                buffer.content,
                                BufferContent::Media {
                                    kind: MediaKind::Document,
                                    ..
                                }
                            ) {
                                if self.readers.contains_key(&id) {
                                    return self.show_reader(id);
                                }
                                let path = match self.buffers.path(buffer.source.as_ref().unwrap())
                                {
                                    Ok(path) => path,
                                    Err(error) => {
                                        self.status = error;
                                        return Task::none();
                                    }
                                };
                                let position = if self.settings.viewer.remember_position {
                                    self.reader_store
                                        .as_ref()
                                        .and_then(|store| store.position(&path).ok())
                                        .unwrap_or_default()
                                } else {
                                    Default::default()
                                };
                                self.readers.insert(id, Viewer::new(position));
                                return self.render_reader(id);
                            }
                        }
                        Err(error) => self.status = error,
                    }
                }
            }
            Message::Activate(id)
                if self.pending_close.is_none()
                    && !self.volume_loading
                    && self.creation_name.is_none()
                    && self.buffers.entries.contains_key(&id) =>
            {
                self.active = Some(id);
                return self.show_reader(id);
            }
            Message::Edit(id, action)
                if self.pending_close.is_none()
                    && self.context_menu.is_none()
                    && self.file_prompt.is_none()
                    && !self.volume_loading
                    && self.creation_name.is_none() =>
            {
                self.active = Some(id);
                if let Some(buffer) = self.buffers.entries.get_mut(&id)
                    && (!buffer.read_only || !action.is_edit())
                    && let Some(editor) = self.editors.get_mut(&id)
                {
                    let edit = action.is_edit();
                    editor.perform(action);
                    if edit {
                        buffer.set_text(editor.text());
                        self.status.clear();
                    }
                }
            }
            Message::Undo | Message::Redo
                if self.pending_close.is_none()
                    && !self.volume_loading
                    && self.creation_name.is_none() =>
            {
                let Some(id) = self.active else {
                    return Task::none();
                };
                let buffer = self.buffers.entries.get_mut(&id).unwrap();
                let previous_tick = buffer.changed_tick;
                if matches!(message, Message::Undo) {
                    buffer.undo();
                } else {
                    buffer.redo();
                }
                if buffer.changed_tick != previous_tick {
                    let mut cursor = self.editors[&id].cursor();
                    let mut editor = text_editor::Content::with_text(buffer.text().unwrap());
                    cursor.position.line = cursor
                        .position
                        .line
                        .min(editor.line_count().saturating_sub(1));
                    // iced columns are UTF-8 byte offsets, including when restoring a cursor.
                    let line = editor.line(cursor.position.line).unwrap();
                    cursor.position.column = cursor.position.column.min(line.text.len());
                    while !line.text.is_char_boundary(cursor.position.column) {
                        cursor.position.column -= 1;
                    }
                    cursor.selection = None;
                    editor.move_to(cursor);
                    self.editors.insert(id, editor);
                    self.status.clear();
                }
            }
            Message::Save | Message::SaveAs
                if !self.busy && self.pending_close.is_none() && self.creation_name.is_none() =>
            {
                let Some(id) = self.active else {
                    return Task::none();
                };
                let buffer = &self.buffers.entries[&id];
                let save_as = matches!(message, Message::SaveAs);
                self.editor_menu = false;
                if buffer.read_only {
                    return Task::none();
                }
                self.busy = true;
                self.status = "Saving...".into();
                if buffer.source.is_none() || save_as {
                    let directory = self.explorer.root.clone();
                    let name = if buffer.source.is_some() {
                        buffer.name()
                    } else {
                        "Untitled.mde".into()
                    };
                    return Task::perform(
                        async move { crate::file_dialog::save(directory, name).await },
                        move |path| Message::SavePath(id, path),
                    );
                }
                return self.save(id, None);
            }
            Message::SavePath(id, path) => {
                self.busy = false;
                match path {
                    Ok(Some(path)) => {
                        self.busy = true;
                        return self.save(id, Some(path));
                    }
                    Ok(None) => self.status.clear(),
                    Err(error) => self.status = error,
                }
            }
            Message::Saved(result) => {
                self.busy = false;
                let saved_id = result.as_ref().ok().map(|saved| saved.id);
                self.status = match result.and_then(|saved| self.buffers.saved(saved)) {
                    Ok(()) => String::new(),
                    Err(error) => error,
                };
                if self.status.is_empty()
                    && let Some(source) = saved_id
                        .and_then(|id| self.buffers.entries.get(&id))
                        .and_then(|buffer| buffer.source.as_ref())
                    && let Ok(file) = self.buffers.path(source)
                    && let Some(parent) = file.parent()
                    && self.explorer.directories.contains_key(parent)
                {
                    let path = parent.to_owned();
                    self.explorer.loading.insert(path.clone());
                    let target = path.clone();
                    return Task::perform(async move { Directory::read(&target) }, move |result| {
                        Message::DirectoryLoaded(path.clone(), result)
                    });
                }
            }
            Message::Close(target) if !self.busy => {
                self.file_prompt = None;
                self.context_menu = None;
                let dirty = match target {
                    CloseTarget::Buffer(id) => {
                        self.buffers.entries.get(&id).is_some_and(|b| b.modified())
                    }
                    CloseTarget::Window => self.buffers.entries.values().any(|b| b.modified()),
                    CloseTarget::SwitchVolume(_) => {
                        self.buffers.entries.values().any(|b| b.modified())
                    }
                };
                if dirty {
                    self.pending_close = Some(target);
                } else {
                    return self.close(target);
                }
            }
            Message::Discard => {
                if let Some(target) = self.pending_close.take() {
                    return self.close(target);
                }
            }
            Message::Cancel => self.pending_close = None,
            _ => {}
        }
        Task::none()
    }

    #[cfg(test)]
    fn new_buffer(&mut self) {
        let id = self.buffers.scratch();
        self.editors.insert(id, text_editor::Content::new());
        self.active = Some(id);
        if self.viewport_width < 600.0 {
            self.sidebar = false;
        }
        self.status.clear();
    }

    fn key_name(key: &keyboard::Key, physical: keyboard::key::Physical) -> String {
        if let Some(key) = key.to_latin(physical) {
            return key.to_ascii_lowercase().to_string();
        }
        match key {
            keyboard::Key::Character(key) => key.to_ascii_lowercase(),
            keyboard::Key::Named(key) => format!("{key:?}").to_ascii_lowercase(),
            keyboard::Key::Unidentified => String::new(),
        }
    }

    fn context_action(&mut self, target: Target, action: ContextAction) -> Task<Message> {
        self.status.clear();
        match action {
            ContextAction::Open => self.update(if target.kind == EntryKind::Folder {
                Message::ToggleFolder(target.path)
            } else {
                Message::OpenFile(target.path)
            }),
            ContextAction::NewFile | ContextAction::NewFolder => {
                let parent = if target.kind == EntryKind::Folder {
                    target.path
                } else {
                    target.path.parent().unwrap().to_owned()
                };
                self.begin_entry(
                    parent,
                    if matches!(action, ContextAction::NewFile) {
                        EntryKind::File
                    } else {
                        EntryKind::Folder
                    },
                )
            }
            ContextAction::Rename => {
                if Some(&target.path) == self.explorer.root.as_ref() {
                    return Task::none();
                }
                let parent = target.path.parent().unwrap().to_owned();
                self.entry_draft = Some(EntryDraft {
                    parent: parent.clone(),
                    kind: target.kind,
                    name: target
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    source: Some(target.path),
                });
                self.explorer.expanded.insert(parent);
                iced::widget::operation::focus("entry-name")
                    .chain(iced::widget::operation::select_all("entry-name"))
            }
            ContextAction::Move => {
                if Some(&target.path) == self.explorer.root.as_ref() {
                    return Task::none();
                }
                let destination = target.path.parent().unwrap().to_owned();
                let directory = Directory {
                    path: destination.clone(),
                    entries: Vec::new(),
                };
                self.file_prompt = Some(FilePrompt::Move {
                    source: target.path,
                    destination: destination.clone(),
                    directory,
                });
                self.update(Message::DestinationRequested(destination))
            }
            ContextAction::Duplicate => {
                self.run_file_operation(FileOperation::Duplicate(target.path))
            }
            ContextAction::CopyPath => {
                iced::clipboard::write(target.path.to_string_lossy().into_owned())
            }
            ContextAction::Delete => {
                if Some(&target.path) == self.explorer.root.as_ref() {
                    return Task::none();
                }
                let modified = self
                    .buffers
                    .affected(&target.path)
                    .iter()
                    .any(|id| self.buffers.entries[id].modified());
                self.file_prompt = Some(FilePrompt::Delete {
                    source: target.path,
                    modified,
                });
                Task::none()
            }
            ContextAction::Restore => {
                if let Some(record) = self.last_deleted.clone() {
                    self.run_file_operation(FileOperation::Restore(record))
                } else {
                    Task::none()
                }
            }
            ContextAction::Refresh => {
                let path = if target.kind == EntryKind::Folder {
                    target.path
                } else {
                    target.path.parent().unwrap().to_owned()
                };
                self.explorer.loading.insert(path.clone());
                let requested = path.clone();
                Task::perform(async move { Directory::read(&requested) }, move |result| {
                    Message::DirectoryLoaded(path.clone(), result)
                })
            }
            ContextAction::CollapseAll => {
                self.explorer.expanded.clear();
                if let Some(root) = &self.explorer.root {
                    self.explorer.expanded.insert(root.clone());
                }
                Task::none()
            }
        }
    }

    fn run_file_operation(&mut self, operation: FileOperation) -> Task<Message> {
        let Some(volume) = self.volume.clone() else {
            return Task::none();
        };
        self.busy = true;
        self.context_menu = None;
        self.file_prompt = None;
        self.drag = None;
        self.status.clear();
        Task::perform(
            async move { operation.run(&volume) },
            Message::FileCompleted,
        )
    }

    fn apply_file_outcome(&mut self, outcome: FileOutcome) {
        let mut warnings = Vec::new();
        self.entry_draft = None;
        if let Some(change) = &outcome.change {
            if let Some(store) = &self.reader_store
                && let Err(error) = store.relocate(change)
            {
                warnings.push(error);
            }
            if let Err(error) = self.buffers.relocate(change) {
                warnings.push(error);
            }
            self.explorer.relocate(change);
            if let Some(folder) = &self.selected_folder
                && let Ok(suffix) = folder.strip_prefix(&change.from)
            {
                self.selected_folder = Some(change.to.join(suffix));
            }
        }
        if let Some(removed) = &outcome.removed {
            for id in self.buffers.affected(removed) {
                let _ = self.buffers.close(id, true);
                self.editors.remove(&id);
                self.images.remove(&id);
                self.readers.remove(&id);
                if self.active == Some(id) {
                    self.active = None;
                }
            }
            if self.active.is_none() {
                self.active = self.buffers.entries.keys().next().copied();
            }
            self.explorer.remove_subtree(removed);
        }
        for directory in outcome.refreshed {
            match directory {
                Ok(directory) => {
                    self.explorer.loading.remove(&directory.path);
                    self.explorer.expanded.insert(directory.path.clone());
                    self.explorer
                        .directories
                        .insert(directory.path, directory.entries);
                }
                Err(error) => warnings.push(error),
            }
        }
        self.selected_entry = Some(outcome.selected.clone());
        self.selected_folder = if outcome.selected.is_dir() {
            Some(outcome.selected)
        } else {
            outcome.selected.parent().map(Path::to_owned)
        };
        self.hovered_folder = None;
        self.hovered_target = None;
        match outcome.trash {
            Ok(record) => self.last_deleted = record,
            Err(error) => warnings.push(error),
        }
        self.status = warnings.join("\n");
    }

    fn shortcut(&self, key: &str, modifiers: keyboard::Modifiers) -> Option<ShortcutAction> {
        self.settings
            .shortcuts
            .action(
                key,
                ShortcutModifiers {
                    control: modifiers.control(),
                    meta: modifiers.logo(),
                    alt: modifiers.alt(),
                    shift: modifiers.shift(),
                },
            )
            .filter(|action| {
                !matches!(
                    action,
                    ShortcutAction::ReaderNextPage | ShortcutAction::ReaderPreviousPage
                ) || self.active.is_some_and(|id| self.readers.contains_key(&id))
            })
    }

    fn begin_entry(&mut self, parent: PathBuf, kind: EntryKind) -> Task<Message> {
        let Some(volume) = &self.volume else {
            return Task::none();
        };
        if parent != volume.root
            && let Err(error) = volume.document_path(&parent)
        {
            self.status = error;
            return Task::none();
        }
        let base = if kind == EntryKind::File {
            "Untitled"
        } else {
            "New folder"
        };
        let mut name = base.to_string();
        let mut suffix = 1;
        while self
            .explorer
            .directories
            .get(&parent)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .name
                        .eq_ignore_ascii_case(&if kind == EntryKind::File {
                            format!("{name}.mde")
                        } else {
                            name.clone()
                        })
                })
            })
        {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        self.entry_draft = Some(EntryDraft {
            parent: parent.clone(),
            kind,
            name,
            source: None,
        });
        self.selected_folder = Some(parent.clone());
        self.explorer.expanded.insert(parent.clone());
        self.status.clear();
        let focus = iced::widget::operation::focus("entry-name")
            .chain(iced::widget::operation::select_all("entry-name"));
        if !self.explorer.directories.contains_key(&parent)
            && self.explorer.loading.insert(parent.clone())
        {
            let path = parent.clone();
            return Task::batch([
                focus,
                Task::perform(async move { Directory::read(&path) }, move |result| {
                    Message::DirectoryLoaded(parent.clone(), result)
                }),
            ]);
        }
        focus
    }

    fn begin_volume(&mut self, action: VolumeAction) -> Task<Message> {
        let flush = self.flush_readers();
        self.status.clear();
        match action {
            VolumeAction::Create => {
                self.creation_name = Some(String::new());
                flush
            }
            VolumeAction::Open => {
                self.creation_name = None;
                self.busy = true;
                self.volume_loading = true;
                let base = self.base_settings.clone();
                flush.chain(Task::perform(
                    async move {
                        match crate::file_dialog::folder().await {
                            Ok(Some(path)) => Some(OpenedVolume::open(&path, &base)),
                            Ok(None) => None,
                            Err(error) => Some(Err(error)),
                        }
                    },
                    Message::VolumeOpened,
                ))
            }
        }
    }

    fn apply_volume(&mut self, volume: OpenedVolume) -> Result<(), String> {
        let workspace = volume.workspace;
        let restored_files = volume.restored_files;
        self.last_deleted = volume.trash.clone();
        let mut buffers = Buffers::default();
        buffers.mount(&volume.storage.root)?;
        let mut restored_ids = BTreeMap::new();
        for (index, file) in restored_files {
            if let Ok(id) = buffers.insert(file) {
                restored_ids.insert(index, id);
            }
        }
        self.settings = volume.settings.settings;
        self.status = volume
            .settings
            .warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        self.sidebar = self.settings.workspace.sidebar_visible;

        self.buffers = buffers;
        self.editors.clear();
        self.images.clear();
        self.readers.clear();
        self.reader_store = match ReaderStore::load(&volume.storage) {
            Ok(store) => Some(store),
            Err(error) => {
                self.status.push_str(&format!("\nViewer state: {error}"));
                None
            }
        };
        self.active = None;
        self.clear_tab_drag();
        self.tab_order = restored_ids.values().copied().collect();
        (self.panes, self.focused_pane) = pane_grid::State::new(None);
        self.explorer.set_root(volume.directory);
        self.volume = Some(volume.storage);
        self.creation_name = None;
        self.entry_draft = None;
        self.hovered_folder = None;
        self.editor_menu = false;
        self.selected_folder = Some(self.volume.as_ref().unwrap().root.clone());
        self.selected_entry = self.selected_folder.clone();
        self.context_menu = None;
        self.file_prompt = None;
        self.hovered_target = None;
        self.drag = None;
        self.restore_workspace(workspace, &restored_ids);
        Ok(())
    }

    fn restore_workspace(&mut self, workspace: WorkspaceSession, ids: &BTreeMap<usize, BufferId>) {
        fn configuration(
            layout: PaneLayout,
            ids: &BTreeMap<usize, BufferId>,
        ) -> pane_grid::Configuration<Option<BufferId>> {
            match layout {
                PaneLayout::Pane { tab } => {
                    pane_grid::Configuration::Pane(tab.and_then(|index| ids.get(&index).copied()))
                }
                PaneLayout::Split {
                    vertical,
                    ratio,
                    a,
                    b,
                } => pane_grid::Configuration::Split {
                    axis: if vertical {
                        pane_grid::Axis::Vertical
                    } else {
                        pane_grid::Axis::Horizontal
                    },
                    ratio,
                    a: Box::new(configuration(*a, ids)),
                    b: Box::new(configuration(*b, ids)),
                },
            }
        }
        for (&id, buffer) in &self.buffers.entries {
            if let Some(text) = buffer.text() {
                self.editors
                    .insert(id, text_editor::Content::with_text(text));
            }
            if let BufferContent::Media {
                kind,
                bytes: Some(bytes),
                ..
            } = &buffer.content
            {
                match kind {
                    MediaKind::Image => {
                        self.images
                            .insert(id, image::Handle::from_bytes(bytes.to_vec()));
                    }
                    MediaKind::Document => {
                        let position = if self.settings.viewer.remember_position {
                            buffer
                                .source
                                .as_ref()
                                .and_then(|source| self.buffers.path(source).ok())
                                .and_then(|path| {
                                    self.reader_store
                                        .as_ref()
                                        .and_then(|store| store.position(&path).ok())
                                })
                                .unwrap_or_default()
                        } else {
                            Default::default()
                        };
                        self.readers.insert(id, Viewer::new(position));
                    }
                    _ => {}
                }
            }
        }
        self.panes = pane_grid::State::with_configuration(configuration(workspace.layout, ids));
        self.active = workspace.active.and_then(|index| ids.get(&index).copied());
        if workspace.active.is_some() && self.active.is_none() {
            self.active = ids.values().next().copied();
        }
        self.focused_pane = self
            .panes
            .iter()
            .find_map(|(pane, value)| (*value == self.active).then_some(*pane))
            .unwrap_or_else(|| *self.panes.iter().next().unwrap().0);
        *self.panes.get_mut(self.focused_pane).unwrap() = self.active;
    }

    fn sync_tab_order(&mut self) {
        self.tab_order
            .retain(|id| self.buffers.entries.contains_key(id));
        for id in self.buffers.entries.keys() {
            if !self.tab_order.contains(id) {
                self.tab_order.push(*id);
            }
        }
    }

    fn clear_tab_drag(&mut self) {
        self.tab_drag = None;
        self.tab_hover = None;
        self.tab_pane_hover = None;
    }

    fn reorder_tab(&mut self, source: BufferId, target: BufferId, before: bool) {
        if source == target || !self.tab_order.contains(&target) {
            return;
        }
        let Some(from) = self.tab_order.iter().position(|id| *id == source) else {
            return;
        };
        self.tab_order.remove(from);
        let to = self.tab_order.iter().position(|id| *id == target).unwrap() + usize::from(!before);
        self.tab_order.insert(to, source);
    }

    fn drop_tab(
        &mut self,
        buffer: BufferId,
        target: pane_grid::Pane,
        region: pane_grid::Region,
    ) -> Task<Message> {
        if !self.buffers.entries.contains_key(&buffer) || self.panes.get(target).is_none() {
            return Task::none();
        }
        let source = self
            .panes
            .iter()
            .find_map(|(pane, value)| (*value == Some(buffer)).then_some(*pane));
        match region {
            pane_grid::Region::Center => {
                let displaced = *self.panes.get(target).unwrap();
                if let Some(source) = source
                    && source != target
                {
                    *self.panes.get_mut(source).unwrap() = displaced;
                }
                *self.panes.get_mut(target).unwrap() = Some(buffer);
                self.focused_pane = target;
            }
            pane_grid::Region::Edge(edge) => {
                if let Some(source) = source
                    && source != target
                {
                    self.panes
                        .drop(source, pane_grid::Target::Pane(target, region));
                    self.focused_pane = source;
                } else {
                    let axis = match edge {
                        pane_grid::Edge::Left | pane_grid::Edge::Right => pane_grid::Axis::Vertical,
                        _ => pane_grid::Axis::Horizontal,
                    };
                    if self.panes.len() >= 64 {
                        return Task::none();
                    }
                    let Some((new, _)) = self.panes.split(axis, target, Some(buffer)) else {
                        return Task::none();
                    };
                    if source == Some(target) {
                        *self.panes.get_mut(target).unwrap() = None;
                    }
                    if matches!(edge, pane_grid::Edge::Left | pane_grid::Edge::Top) {
                        self.panes.swap(new, target);
                    }
                    self.focused_pane = new;
                }
            }
        }
        self.active = Some(buffer);
        Task::batch([self.resize_readers(), self.show_reader(buffer)])
    }

    fn workspace_snapshot(&self) -> WorkspaceSession {
        let mut tabs = Vec::new();
        let mut indices = BTreeMap::new();
        for &id in &self.tab_order {
            let Some(buffer) = self.buffers.entries.get(&id) else {
                continue;
            };
            if let Some(source) = &buffer.source {
                indices.insert(id, tabs.len());
                tabs.push(source.path.clone());
            }
        }
        fn layout(
            node: &pane_grid::Node,
            panes: &pane_grid::State<Option<BufferId>>,
            indices: &BTreeMap<BufferId, usize>,
        ) -> PaneLayout {
            match node {
                pane_grid::Node::Pane(pane) => PaneLayout::Pane {
                    tab: panes
                        .get(*pane)
                        .copied()
                        .flatten()
                        .and_then(|id| indices.get(&id).copied()),
                },
                pane_grid::Node::Split {
                    axis, ratio, a, b, ..
                } => PaneLayout::Split {
                    vertical: *axis == pane_grid::Axis::Vertical,
                    ratio: *ratio,
                    a: Box::new(layout(a, panes, indices)),
                    b: Box::new(layout(b, panes, indices)),
                },
            }
        }
        WorkspaceSession {
            version: 1,
            tabs,
            active: self.active.and_then(|id| indices.get(&id).copied()),
            layout: layout(self.panes.layout(), &self.panes, &indices),
        }
    }

    fn schedule_session_save(&mut self) -> Task<Message> {
        if self.session_pending || (self.session.is_none() && self.volume.is_none()) {
            return Task::none();
        }
        self.session_pending = true;
        let (send, receive) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = send.send(());
        });
        Task::perform(
            async move {
                let _ = receive.await;
            },
            |_| Message::PersistSession,
        )
    }

    fn flush_session(&mut self) -> Task<Message> {
        let root = self.volume.as_ref().map(|volume| volume.root.clone());
        let workspace = self.workspace_snapshot();
        let window = WindowPlacement {
            width: self.viewport_width,
            height: self.viewport_height,
            position: self.window_position,
        };
        let session = self.session.clone();
        self.session_revision += 1;
        let revision = self.session_revision;
        let io = self.session_io.clone();
        let (send, receive) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let save = || -> Result<(), String> {
                let mut latest = io.lock().map_err(|_| "Session state lock poisoned")?;
                if revision < *latest {
                    return Ok(());
                }
                *latest = revision;
                let mut errors = Vec::new();
                if let Some(root) = root
                    && let Err(error) = workspace.save(&root)
                {
                    errors.push(error);
                }
                if window.valid()
                    && let Some(session) = session
                    && let Err(error) = session.remember_window(&window)
                {
                    errors.push(error);
                }
                if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("\n"))
                }
            };
            let _ = send.send(save());
        });
        Task::perform(
            async move {
                receive
                    .await
                    .unwrap_or_else(|_| Err("Session save stopped".into()))
            },
            Message::SessionSaved,
        )
    }

    fn reader_width(&self) -> f32 {
        (self.viewport_width
            - 38.0
            - if self.sidebar && self.viewport_width >= 600.0 {
                f32::from(self.settings.workspace.sidebar_width) + 5.0
            } else {
                0.0
            }
            - 24.0)
            .max(160.0)
    }

    fn reader_width_for(&self, id: BufferId) -> f32 {
        let pane = self
            .panes
            .iter()
            .find_map(|(pane, value)| (*value == Some(id)).then_some(*pane))
            .unwrap_or(self.focused_pane);
        self.panes
            .layout()
            .pane_regions(
                4.0,
                100.0,
                iced::Size::new(self.reader_width() + 24.0, self.viewport_height),
            )
            .get(&pane)
            .map(|bounds| (bounds.width - 24.0).max(80.0))
            .unwrap_or(self.reader_width())
    }

    fn resize_readers(&mut self) -> Task<Message> {
        let ids: Vec<_> = self
            .panes
            .iter()
            .filter_map(|(_, id)| *id)
            .filter(|id| self.readers.contains_key(id))
            .collect();
        let mut tasks = Vec::new();
        for id in ids {
            let width = self.reader_width_for(id);
            let viewer = self.readers.get_mut(&id).unwrap();
            viewer.keep_scroll = false;
            if viewer.image.is_none()
                || (viewer.display_width - width * viewer.position.zoom).abs() > 2.0
            {
                tasks.push(self.render_reader(id));
            }
        }
        Task::batch(tasks)
    }

    fn render_reader(&mut self, id: BufferId) -> Task<Message> {
        let Some(buffer) = self.buffers.entries.get(&id) else {
            return Task::none();
        };
        let BufferContent::Media {
            kind: MediaKind::Document,
            bytes: Some(bytes),
            ..
        } = &buffer.content
        else {
            return Task::none();
        };
        let bytes = bytes.clone();
        let extension = buffer
            .source
            .as_ref()
            .unwrap()
            .path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let available = self.reader_width_for(id);
        let viewer = self.readers.get_mut(&id).unwrap();
        self.reader_request += 1;
        let request = self.reader_request;
        viewer.request = request;
        viewer.restoring = !viewer.keep_scroll;
        viewer.error.clear();
        viewer.display_width = available * viewer.position.zoom;
        // Reuse a nearby raster size during small resizes; image layout stays exact.
        viewer.pixel_width =
            ((viewer.display_width * 2.0 / 64.0).ceil() * 64.0).clamp(256.0, 2400.0);
        viewer.page_window =
            ((self.viewport_height / viewer.slot_height()).ceil() as u32 + 3).clamp(4, 12);
        let page = viewer.position.page;
        let pixel_width = viewer.pixel_width;
        viewer.loading = viewer.cached_page(page).is_none();
        let worker = viewer
            .worker
            .get_or_insert_with(|| enote::reader::RenderWorker::new(bytes, extension));
        worker.set_generation(request);
        let mut candidates = vec![page];
        // Visible/forward pages first, then a small buffer behind the viewport.
        candidates.extend(page.saturating_add(1)..page.saturating_add(viewer.page_window));
        candidates.extend((page.saturating_sub(2)..page).rev());
        let mut tasks = Vec::new();
        for candidate in candidates {
            if viewer.count.is_some_and(|count| candidate >= count) {
                continue;
            }
            let cached = viewer.cached_page(candidate).cloned();
            if candidate == page {
                if let Some(cached) = cached {
                    tasks.push(Task::done(Message::ReaderRendered(id, request, Ok(cached))));
                } else {
                    tasks.push(Task::perform(
                        viewer
                            .worker
                            .as_ref()
                            .unwrap()
                            .render(request, candidate, pixel_width),
                        move |result| Message::ReaderRendered(id, request, result),
                    ));
                }
            } else if cached.is_none() {
                tasks.push(Task::perform(
                    viewer
                        .worker
                        .as_ref()
                        .unwrap()
                        .render(request, candidate, pixel_width),
                    move |result| Message::ReaderPrefetched(id, request, result),
                ));
            }
        }
        Task::batch(tasks)
    }

    fn show_reader(&mut self, id: BufferId) -> Task<Message> {
        if self.readers.get(&id).is_some_and(|viewer| {
            !viewer.loading
                && (viewer.image.is_none()
                    || (viewer.display_width - self.reader_width_for(id) * viewer.position.zoom)
                        .abs()
                        > 2.0)
        }) {
            self.readers.get_mut(&id).unwrap().keep_scroll = false;
            self.render_reader(id)
        } else {
            self.restore_reader(id)
        }
    }

    fn restore_reader(&mut self, id: BufferId) -> Task<Message> {
        let Some(viewer) = self.readers.get_mut(&id) else {
            return Task::none();
        };
        if viewer.loading || viewer.image.is_none() {
            return Task::none();
        }
        viewer.restoring = true;
        let request = viewer.request;
        iced::widget::operation::scroll_to(
            Viewer::scroll_id(id),
            scrollable::AbsoluteOffset {
                x: viewer.position.x * viewer.display_width,
                y: (viewer.position.page as f32 + viewer.position.y) * viewer.slot_height(),
            },
        )
        .chain(Task::done(Message::ReaderRestored(id, request)))
    }

    fn remember_reader(&mut self, id: BufferId) -> Task<Message> {
        if !self.settings.viewer.remember_position {
            return Task::none();
        }
        let Some(viewer) = self.readers.get(&id) else {
            return Task::none();
        };
        let Some(source) = self
            .buffers
            .entries
            .get(&id)
            .and_then(|buffer| buffer.source.as_ref())
        else {
            return Task::none();
        };
        if let (Some(store), Ok(path)) = (&self.reader_store, self.buffers.path(source))
            && let Err(error) = store.update(&path, viewer.position)
        {
            self.status = error;
        }
        if self.reader_save_pending || self.reader_store.is_none() {
            return Task::none();
        }
        self.reader_save_pending = true;
        Task::perform(
            async {
                std::thread::sleep(std::time::Duration::from_millis(250));
            },
            |_| Message::PersistReaders,
        )
    }

    fn flush_readers(&self) -> Task<Message> {
        if !self.settings.viewer.remember_position {
            return Task::none();
        }
        if let (Some(store), Some(volume)) = (&self.reader_store, &self.volume) {
            let store = store.clone();
            let identity = volume.manifest.id;
            Task::perform(async move { store.flush() }, move |result| {
                Message::ReadersPersisted(identity, result)
            })
        } else {
            Task::none()
        }
    }

    fn reader_action(&mut self, id: BufferId, action: ReaderAction) -> Task<Message> {
        let Some(viewer) = self.readers.get_mut(&id) else {
            return Task::none();
        };
        match action {
            ReaderAction::Hover(hovered) => {
                viewer.hovered = hovered;
                return Task::none();
            }
            ReaderAction::StreamScroll { request, x, y } => {
                if viewer.request != request || viewer.restoring || !y.is_finite() {
                    return Task::none();
                }
                let slot = viewer.slot_height();
                let page =
                    ((y / slot).floor() as u32).min(viewer.count.unwrap_or(1).saturating_sub(1));
                let changed = viewer.position.page != page;
                viewer.position.page = page;
                viewer.position.y = (y / slot - page as f32).clamp(0.0, 1.0);
                viewer.position.x = if x.is_finite() {
                    (x / viewer.display_width).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                viewer.page_input = (page + 1).to_string();
                viewer.keep_scroll = true;
                let normalized = ReaderAction::Scroll {
                    request,
                    x: viewer.position.x,
                    y: viewer.position.y,
                };
                let save = self.reader_action(id, normalized);
                return if changed {
                    Task::batch([save, self.render_reader(id)])
                } else {
                    save
                };
            }
            ReaderAction::Scroll { request, x, y } => {
                if viewer.request == request && !viewer.restoring {
                    // An axis without overflow has a 0/0 relative offset in iced.
                    viewer.position.x = if x.is_finite() {
                        x.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    viewer.position.y = if y.is_finite() {
                        y.clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    return self.remember_reader(id);
                }
                return Task::none();
            }
            ReaderAction::PageInput(value) => {
                if value.len() <= 8 && value.chars().all(|c| c.is_ascii_digit()) {
                    viewer.page_input = value;
                }
                return Task::none();
            }
            ReaderAction::Retry => {}
            ReaderAction::Previous => {
                if viewer.position.page == 0 {
                    return Task::none();
                }
                viewer.position.page -= 1;
                viewer.position.x = 0.0;
                viewer.position.y = 0.0;
            }
            ReaderAction::Next => {
                if viewer
                    .count
                    .is_none_or(|count| viewer.position.page + 1 >= count)
                {
                    return Task::none();
                }
                viewer.position.page += 1;
                viewer.position.x = 0.0;
                viewer.position.y = 0.0;
            }
            ReaderAction::Go => {
                let Ok(page) = viewer.page_input.parse::<u32>() else {
                    viewer.page_input = (viewer.position.page + 1).to_string();
                    return Task::none();
                };
                let Some(count) = viewer.count else {
                    return Task::none();
                };
                viewer.position.page = page.clamp(1, count) - 1;
                viewer.position.x = 0.0;
                viewer.position.y = 0.0;
            }
            ReaderAction::ZoomIn => viewer.position.zoom = (viewer.position.zoom + 0.25).min(3.0),
            ReaderAction::ZoomOut => viewer.position.zoom = (viewer.position.zoom - 0.25).max(0.5),
            ReaderAction::Fit => {
                viewer.position.zoom = 1.0;
                viewer.position.x = 0.0;
            }
        }
        viewer.page_input = (viewer.position.page + 1).to_string();
        viewer.keep_scroll = false;
        self.render_reader(id)
    }

    fn save(&mut self, id: BufferId, path: Option<PathBuf>) -> Task<Message> {
        if let (Some(volume), Some(path)) = (&self.volume, &path)
            && let Err(error) = volume.document_path(path)
        {
            self.busy = false;
            self.status = error;
            return Task::none();
        }
        match self.buffers.save_job(id, path) {
            Ok(job) => Task::perform(async move { job.write() }, Message::Saved),
            Err(error) => {
                self.busy = false;
                self.status = error;
                Task::none()
            }
        }
    }

    fn close(&mut self, target: CloseTarget) -> Task<Message> {
        match target {
            CloseTarget::Window => self
                .flush_session()
                .chain(self.flush_readers())
                .chain(iced::exit()),
            CloseTarget::SwitchVolume(action) => {
                let flush = self.flush_session().chain(self.flush_readers());
                flush.chain(self.begin_volume(action))
            }
            CloseTarget::Buffer(id) => {
                if let Err(error) = self.buffers.close(id, true) {
                    self.status = error;
                    return Task::none();
                }
                self.editors.remove(&id);
                self.images.remove(&id);
                self.readers.remove(&id);
                if self.active == Some(id) {
                    self.active = self.buffers.entries.keys().next().copied();
                }
                let flush = self.flush_readers();
                if let Some(active) = self.active {
                    Task::batch([flush, self.restore_reader(active)])
                } else {
                    flush
                }
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        if self.pending_close.is_none() && (self.volume.is_none() || self.creation_name.is_some()) {
            return self.launcher_view();
        }
        let mut workspace = column![];
        if !self.buffers.entries.is_empty() {
            let tabs = row(self
                .tab_order
                .iter()
                .filter_map(|id| self.buffers.entries.get(id))
                .map(|buffer| {
                    let id = buffer.id;
                    let active = self.active == Some(id);
                    let label = format!(
                        "{}{}",
                        buffer.name(),
                        if buffer.modified() { " *" } else { "" }
                    );
                    let dragging = self.tab_drag.as_ref().is_some_and(|drag| drag.moving);
                    let indicator = |before| {
                        container(Space::new().width(2).height(22)).style(move |_| {
                            look::surface(if dragging && self.tab_hover == Some((id, before)) {
                                look::MUTED
                            } else {
                                look::CANVAS
                            })
                        })
                    };
                    let label = container(text(label).size(12))
                        .padding([7, 12])
                        .style(move |_| {
                            look::surface(if active { look::PANEL } else { look::CANVAS })
                        });
                    row![
                        indicator(true),
                        DragArea::with_bounds(
                            label,
                            iced::mouse::Interaction::Grab,
                            move |phase, point, bounds| Message::TabPointer(
                                id, phase, point, bounds
                            )
                        ),
                        self.tool(
                            Icon::X,
                            "Close",
                            Message::Close(CloseTarget::Buffer(id)),
                            !self.busy
                        ),
                        indicator(false),
                    ]
                    .spacing(0)
                    .align_y(iced::Center)
                    .into()
                }))
            .spacing(4)
            .padding([4, 8]);
            let writable = self
                .active
                .and_then(|id| self.buffers.entries.get(&id))
                .is_some_and(|buffer| !buffer.read_only);
            workspace = workspace.push(
                row![
                    scrollable(tabs)
                        .direction(scrollable::Direction::Horizontal(
                            scrollable::Scrollbar::default()
                        ))
                        .width(Fill),
                    self.tool(Icon::Save, "Save", Message::Save, writable && !self.busy),
                    self.tool(
                        Icon::Ellipsis,
                        "File actions",
                        Message::ToggleEditorMenu,
                        !self.busy
                    ),
                ]
                .align_y(iced::Center)
                .padding([0, 8]),
            );
            if self.editor_menu {
                workspace = workspace.push(
                    row![
                        Space::new().width(Fill),
                        button(text("Save as...").size(12))
                            .style(look::quiet)
                            .on_press_maybe((writable && !self.busy).then_some(Message::SaveAs)),
                        button(text("Open...").size(12))
                            .style(look::quiet)
                            .on_press_maybe((!self.busy).then_some(Message::Open)),
                        button(text("Close tab").size(12))
                            .style(look::quiet)
                            .on_press_maybe((!self.busy).then_some(Message::CloseActiveTab)),
                    ]
                    .spacing(8)
                    .padding([4, 8]),
                );
            }
        }
        if let Some(target) = self.pending_close {
            let label = match target {
                CloseTarget::Buffer(_) => "Discard unsaved changes in this buffer?",
                CloseTarget::Window => "Discard all unsaved changes and quit?",
                CloseTarget::SwitchVolume(_) => "Discard unsaved changes and switch volume?",
            };
            workspace = workspace.push(
                container(
                    column![
                        text(label).size(13),
                        row![
                            button("Cancel")
                                .style(look::quiet)
                                .on_press(Message::Cancel),
                            button("Discard")
                                .style(button::danger)
                                .on_press(Message::Discard),
                        ]
                        .spacing(8)
                    ]
                    .spacing(10),
                )
                .padding(16),
            );
        }
        workspace = workspace.push(DragArea::tracking(
            pane_grid::PaneGrid::new(&self.panes, |pane, buffer, _| {
                let label = buffer
                    .and_then(|id| self.buffers.entries.get(&id))
                    .map(|b| b.name())
                    .unwrap_or_default();
                let controls = row![
                    self.tool(
                        Icon::Columns2,
                        "Split right",
                        Message::SplitPane(pane, pane_grid::Axis::Vertical),
                        !self.busy
                    ),
                    self.tool(
                        Icon::Rows2,
                        "Split below",
                        Message::SplitPane(pane, pane_grid::Axis::Horizontal),
                        !self.busy
                    ),
                    self.tool(
                        Icon::X,
                        "Close pane",
                        Message::ClosePane(pane),
                        self.panes.len() > 1
                    ),
                ]
                .align_y(iced::Center);
                let header = pane_grid::TitleBar::new(
                    container(
                        text(label)
                            .size(11)
                            .wrapping(iced::widget::text::Wrapping::None),
                    )
                    .padding([4, 8])
                    .width(Fill),
                )
                .controls(pane_grid::Controls::new(controls))
                .always_show_controls()
                .style(move |_| {
                    look::surface(if pane == self.focused_pane {
                        look::PANEL
                    } else {
                        look::CANVAS
                    })
                });
                let body = self.buffer_view_for(*buffer);
                let dragging = self.tab_drag.as_ref().is_some_and(|drag| drag.moving);
                let body: Element<'_, Message> = if dragging {
                    DragArea::drop_target(body, move |phase, point, bounds| {
                        Message::PanePointer(pane, phase, point, bounds)
                    })
                    .into()
                } else {
                    DragArea::observe(body, move |phase, point, bounds| {
                        Message::PanePointer(pane, phase, point, bounds)
                    })
                    .into()
                };
                let body: Element<'_, Message> = if let Some((target, region)) =
                    self.tab_pane_hover.filter(|(target, _)| *target == pane)
                {
                    let _ = target;
                    let hint = match region {
                        pane_grid::Region::Center => "Move tab here",
                        pane_grid::Region::Edge(pane_grid::Edge::Left) => "Split left",
                        pane_grid::Region::Edge(pane_grid::Edge::Right) => "Split right",
                        pane_grid::Region::Edge(pane_grid::Edge::Top) => "Split above",
                        pane_grid::Region::Edge(pane_grid::Edge::Bottom) => "Split below",
                    };
                    stack![
                        body,
                        container(
                            container(text(hint).size(13))
                                .padding(12)
                                .style(look::popup)
                        )
                        .center(Fill)
                    ]
                    .into()
                } else {
                    body
                };
                pane_grid::Content::new(body).title_bar(header)
            })
            .spacing(4)
            .on_click(Message::FocusPane)
            .on_drag(Message::DragPane)
            .min_size(100)
            .on_resize(6, Message::ResizePane)
            .height(Fill),
        ));
        let narrow = self.viewport_width < 600.0;
        let show_sidebar = self.sidebar && (!narrow || self.pending_close.is_none());
        let mut layout = row![];
        if !narrow || !show_sidebar {
            layout = layout.push(
                container(workspace)
                    .style(|_| look::surface(look::CANVAS))
                    .width(Fill)
                    .height(Fill),
            );
        }
        if show_sidebar {
            if !narrow {
                layout = layout.push(DragArea::new(
                    container(Space::new().height(Fill))
                        .width(5)
                        .style(|_| look::surface(look::LINE)),
                    iced::mouse::Interaction::ResizingHorizontally,
                    Message::ResizeSidebar,
                ));
            }
            layout = layout.push(
                container(self.file_panel())
                    .width(if narrow {
                        iced::Length::Fill
                    } else {
                        iced::Length::Fixed(self.settings.workspace.sidebar_width as f32)
                    })
                    .height(Fill)
                    .style(|_| look::surface(look::PANEL)),
            );
        }
        let rail = container(
            column![self.tool(Icon::Files, "Files", Message::ToggleSidebar, true)].padding([8, 3]),
        )
        .width(38)
        .height(Fill)
        .style(|_| look::surface(look::RAIL));
        let base = layout.push(rail).height(Fill).into();
        self.file_overlays(base)
    }

    fn file_overlays<'a>(&'a self, base: Element<'a, Message>) -> Element<'a, Message> {
        let mut layers = stack![base];
        if let Some(drag) = &self.tab_drag
            && drag.moving
            && let Some(buffer) = self.buffers.entries.get(&drag.buffer)
        {
            layers = layers.push(
                container(
                    container(text(buffer.name()).size(12))
                        .padding(8)
                        .width(190)
                        .style(look::popup),
                )
                .width(Fill)
                .height(Fill)
                .padding(iced::Padding {
                    left: (self.cursor.x + 12.0).clamp(0.0, (self.viewport_width - 198.0).max(0.0)),
                    top: (self.cursor.y + 12.0).clamp(0.0, (self.viewport_height - 52.0).max(0.0)),
                    ..Default::default()
                }),
            );
        }
        if let Some(drag) = &self.drag
            && drag.moving
        {
            let name = drag
                .source
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let label = if name.chars().count() > 24 {
                format!("{}...", name.chars().take(24).collect::<String>())
            } else {
                name.into_owned()
            };
            layers = layers.push(
                container(
                    container(
                        row![
                            look::icon(if drag.source.kind == EntryKind::Folder {
                                Icon::Folder
                            } else {
                                Icon::FileText
                            }),
                            text(label).size(12)
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    )
                    .padding(8)
                    .width(190)
                    .style(look::popup),
                )
                .width(Fill)
                .height(Fill)
                .padding(iced::Padding {
                    left: (self.cursor.x + 12.0).clamp(0.0, (self.viewport_width - 198.0).max(0.0)),
                    top: (self.cursor.y + 12.0).clamp(0.0, (self.viewport_height - 52.0).max(0.0)),
                    right: 0.0,
                    bottom: 0.0,
                }),
            );
        }
        if let Some(menu) = &self.context_menu {
            let root = Some(&menu.target.path) == self.explorer.root.as_ref();
            let folder = menu.target.kind == EntryKind::Folder;
            let mut actions = vec![(
                if folder {
                    Icon::FolderOpen
                } else {
                    Icon::FileText
                },
                if folder { "Expand / collapse" } else { "Open" },
                ContextAction::Open,
                true,
            )];
            if folder {
                actions.extend([
                    (Icon::FilePlus, "New file", ContextAction::NewFile, true),
                    (
                        Icon::FolderPlus,
                        "New folder",
                        ContextAction::NewFolder,
                        true,
                    ),
                ]);
            }
            if !root {
                actions.extend([
                    (Icon::Pencil, "Rename", ContextAction::Rename, true),
                    (Icon::FolderInput, "Move to...", ContextAction::Move, true),
                    (Icon::Copy, "Duplicate", ContextAction::Duplicate, true),
                ]);
            }
            actions.push((Icon::Link, "Copy path", ContextAction::CopyPath, true));
            if folder {
                actions.extend([
                    (Icon::RefreshCw, "Refresh", ContextAction::Refresh, true),
                    (
                        Icon::ChevronsDownUp,
                        "Collapse all",
                        ContextAction::CollapseAll,
                        true,
                    ),
                ]);
            }
            if root {
                actions.push((
                    Icon::Undo2,
                    "Restore last deleted",
                    ContextAction::Restore,
                    self.last_deleted.is_some(),
                ));
            } else {
                actions.push((Icon::Trash2, "Move to trash", ContextAction::Delete, true));
            }
            let height =
                (actions.len() as f32 * 32.0 + 8.0).min((self.viewport_height - 16.0).max(64.0));
            let x = menu
                .position
                .x
                .clamp(8.0, (self.viewport_width - 218.0).max(8.0));
            let y = menu
                .position
                .y
                .clamp(8.0, (self.viewport_height - height - 8.0).max(8.0));
            let items = column(actions.into_iter().map(|(icon, label, action, enabled)| {
                button(
                    row![look::icon(icon), text(label).size(13)]
                        .spacing(10)
                        .align_y(iced::Center),
                )
                .width(Fill)
                .height(32)
                .padding([6, 10])
                .style(look::quiet)
                .on_press_maybe(enabled.then_some(Message::ContextAction(action)))
                .into()
            }))
            .padding(4);
            layers = layers.push(
                mouse_area(Space::new().width(Fill).height(Fill))
                    .on_press(Message::DismissContext)
                    .on_right_press(Message::DismissContext),
            );
            layers = layers.push(
                container(opaque(
                    container(scrollable(items).height(height))
                        .width(210)
                        .style(look::popup),
                ))
                .width(Fill)
                .height(Fill)
                .padding(iced::Padding {
                    left: x,
                    top: y,
                    right: 0.0,
                    bottom: 0.0,
                }),
            );
        }
        if let Some(prompt) = &self.file_prompt {
            let mut content = column![].spacing(12);
            let (label, enabled) = match prompt {
                FilePrompt::Delete { source, modified } => {
                    content = content.push(text("Move to trash?").size(17)).push(
                        text(
                            source
                                .strip_prefix(self.explorer.root.as_ref().unwrap())
                                .unwrap_or(source)
                                .display()
                                .to_string(),
                        )
                        .size(13),
                    );
                    if *modified {
                        content = content.push(
                            text("Unsaved changes in open documents will be discarded.").size(13),
                        );
                    }
                    ("Move to trash", !self.busy)
                }
                FilePrompt::Move {
                    source,
                    destination,
                    directory,
                } => {
                    content = content.push(text("Move to folder").size(17)).push(
                        text(
                            source
                                .file_name()
                                .unwrap_or(source.as_os_str())
                                .to_string_lossy()
                                .into_owned(),
                        )
                        .size(13)
                        .color(look::MUTED),
                    );
                    let root = self.explorer.root.as_ref().unwrap();
                    let breadcrumb = destination.strip_prefix(root).unwrap_or(destination);
                    let breadcrumb = if breadcrumb.as_os_str().is_empty() {
                        root.file_name()
                            .unwrap_or(root.as_os_str())
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        breadcrumb.display().to_string()
                    };
                    content = content.push(
                        row![
                            self.tool(
                                Icon::ArrowUp,
                                "Parent folder",
                                Message::DestinationRequested(
                                    destination.parent().unwrap_or(root).to_owned()
                                ),
                                destination != root && !self.destination_loading
                            ),
                            text(breadcrumb).size(13).width(Fill)
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    );
                    let folders =
                        column(
                            directory
                                .entries
                                .iter()
                                .filter(|entry| entry.directory && !entry.path.starts_with(source))
                                .map(|entry| {
                                    button(
                                        row![
                                            look::icon(Icon::Folder),
                                            text(&entry.name).size(13).width(Fill),
                                            look::icon(Icon::ChevronRight)
                                        ]
                                        .spacing(8)
                                        .align_y(iced::Center),
                                    )
                                    .padding(8)
                                    .width(Fill)
                                    .style(look::quiet)
                                    .on_press_maybe((!self.destination_loading).then_some(
                                        Message::DestinationRequested(entry.path.clone()),
                                    ))
                                    .into()
                                }),
                        );
                    content = content.push(
                        scrollable(folders)
                            .height((self.viewport_height - 270.0).clamp(60.0, 240.0)),
                    );
                    if self.destination_loading {
                        content = content.push(text("Loading...").size(12).color(look::MUTED));
                    }
                    (
                        "Move",
                        !self.busy
                            && !self.destination_loading
                            && source.parent() != Some(destination.as_path())
                            && !destination.starts_with(source),
                    )
                }
            };
            if !self.status.is_empty() {
                content = content.push(text(&self.status).size(12).color(look::MUTED));
            }
            content = content.push(
                row![
                    Space::new().width(Fill),
                    button(text("Cancel").size(13))
                        .style(look::quiet)
                        .on_press_maybe((!self.busy).then_some(Message::CancelFilePrompt)),
                    button(text(label).size(13))
                        .padding([7, 12])
                        .on_press_maybe(enabled.then_some(Message::ConfirmFilePrompt))
                ]
                .spacing(8),
            );
            layers = layers.push(
                mouse_area(container(Space::new().width(Fill).height(Fill)).style(|_| {
                    container::Style {
                        background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4).into()),
                        ..Default::default()
                    }
                }))
                .on_press(Message::CancelFilePrompt),
            );
            layers = layers.push(
                container(opaque(
                    container(scrollable(content))
                        .padding(18)
                        .width(Fill)
                        .max_width(440)
                        .max_height((self.viewport_height - 48.0).max(120.0))
                        .style(look::popup),
                ))
                .padding(24)
                .center(Fill),
            );
        }
        layers.into()
    }

    fn tool(
        &self,
        icon: Icon,
        label: &'static str,
        message: Message,
        enabled: bool,
    ) -> Element<'_, Message> {
        tooltip(
            button(container(look::icon(icon)).center(28))
                .padding(0)
                .style(look::quiet)
                .on_press_maybe((enabled && self.pending_close.is_none()).then_some(message)),
            text(label).size(12),
            tooltip::Position::Bottom,
        )
        .into()
    }

    fn buffer_view_for(&self, buffer: Option<BufferId>) -> Element<'_, Message> {
        let Some(id) = buffer else {
            return Space::new().width(Fill).height(Fill).into();
        };
        let buffer = &self.buffers.entries[&id];
        match &buffer.content {
            BufferContent::Media {
                kind: MediaKind::Document,
                ..
            } => {
                if let Some(viewer) = self.readers.get(&id) {
                    viewer.view(id, self.pending_close.is_none() && !self.busy)
                } else {
                    container(text("Loading...").size(13)).center(Fill).into()
                }
            }
            BufferContent::Text { .. } => {
                let editor = text_editor(&self.editors[&id])
                    .id(if self.active == Some(id) {
                        "active-editor".to_string()
                    } else {
                        format!("editor-{}", id.0)
                    })
                    .height(Fill)
                    .padding(0)
                    .size(f32::from(self.settings.editor.font_size))
                    .style(look::editor)
                    .key_binding(move |press| {
                        if self
                            .shortcut(
                                &Self::key_name(&press.key, press.physical_key),
                                press.modifiers,
                            )
                            .is_some()
                        {
                            None
                        } else {
                            text_editor::Binding::from_key_press(press)
                        }
                    });
                let editor: Element<'_, Message> =
                    if self.pending_close.is_none() && !self.volume_loading {
                        editor
                            .on_action(move |action| Message::Edit(id, action))
                            .into()
                    } else {
                        editor.into()
                    };
                container(container(editor).max_width(820).width(Fill))
                    .padding(if self.viewport_width < 600.0 { 20 } else { 40 })
                    .center_x(Fill)
                    .height(Fill)
                    .into()
            }
            BufferContent::Media {
                kind: MediaKind::Image,
                ..
            } => container(
                image(self.images[&id].clone())
                    .width(Fill)
                    .height(Fill)
                    .content_fit(iced::ContentFit::Contain),
            )
            .padding(24)
            .width(Fill)
            .height(Fill)
            .into(),
            BufferContent::Media { kind, size, .. } => container(
                column![
                    text(buffer.name()).size(18),
                    text(format!("{} / {} bytes", kind.label(), size))
                        .size(12)
                        .color(look::MUTED),
                    text("Preview unavailable").size(12).color(look::MUTED)
                ]
                .spacing(10),
            )
            .center(Fill)
            .into(),
        }
    }

    fn launcher_view(&self) -> Element<'_, Message> {
        let mut content = column![text("eNote").size(28)].spacing(24);
        if let Some(name) = &self.creation_name {
            content = content
                .push(text("New volume").size(16))
                .push(
                    text_input("Volume name", name)
                        .on_input(Message::CreateName)
                        .on_submit(Message::ConfirmCreate)
                        .padding(10),
                )
                .push(
                    row![
                        button("Cancel")
                            .style(look::quiet)
                            .on_press_maybe((!self.busy).then_some(Message::CancelCreate)),
                        button(
                            row![look::icon(Icon::FolderPlus), text("Create").size(13)]
                                .spacing(10)
                                .align_y(iced::Center)
                        )
                        .padding([10, 14])
                        .on_press_maybe(
                            (!self.busy && !name.trim().is_empty())
                                .then_some(Message::ConfirmCreate)
                        )
                    ]
                    .spacing(10),
                );
        } else {
            content = content
                .push(
                    button(
                        row![look::icon(Icon::FolderPlus), text("Create volume").size(14)]
                            .spacing(12)
                            .align_y(iced::Center),
                    )
                    .style(look::selected)
                    .padding(14)
                    .width(Fill)
                    .on_press_maybe((!self.busy).then_some(Message::CreateVolume)),
                )
                .push(
                    button(
                        row![look::icon(Icon::FolderOpen), text("Open volume").size(14)]
                            .spacing(12)
                            .align_y(iced::Center),
                    )
                    .style(look::selected)
                    .padding(14)
                    .width(Fill)
                    .on_press_maybe((!self.busy).then_some(Message::OpenVolume)),
                );
        }
        if !self.status.is_empty() {
            content = content.push(text(&self.status).size(12).color(look::MUTED));
        }
        container(container(content).width(Fill).max_width(340))
            .padding(24)
            .center(Fill)
            .style(|_| look::surface(look::CANVAS))
            .into()
    }

    fn file_panel(&self) -> Element<'_, Message> {
        let enabled = !self.busy;
        let mut panel = column![
            row![
                text("Files").size(12).color(look::MUTED),
                Space::new().width(Fill),
                self.tool(
                    Icon::FolderOpen,
                    "Open volume",
                    Message::OpenVolume,
                    enabled
                ),
            ]
            .align_y(iced::Center)
            .spacing(2)
            .padding([6, 10])
        ];
        if let Some(root) = &self.explorer.root {
            let name = root
                .file_name()
                .unwrap_or(root.as_os_str())
                .to_string_lossy();
            panel = panel.push(self.folder_row(root, name.into_owned(), 0));
        }
        let mut tree = column![];
        if self.entry_draft.as_ref().is_some_and(|draft| {
            draft.source.is_none() && Some(&draft.parent) == self.explorer.root.as_ref()
        }) {
            tree = tree.push(self.entry_input(1));
        }
        let selected_path = self
            .active
            .and_then(|id| self.buffers.entries[&id].source.as_ref())
            .and_then(|source| self.buffers.path(source).ok());
        for (depth, entry) in self.explorer.visible() {
            if self
                .entry_draft
                .as_ref()
                .is_some_and(|draft| draft.source.as_ref() == Some(&entry.path))
            {
                tree = tree.push(self.entry_input(depth + 1));
                continue;
            }
            if entry.directory {
                tree = tree.push(self.folder_row(&entry.path, entry.name.clone(), depth + 1));
                if self
                    .entry_draft
                    .as_ref()
                    .is_some_and(|draft| draft.source.is_none() && draft.parent == entry.path)
                {
                    tree = tree.push(self.entry_input(depth + 2));
                }
                continue;
            }
            let selected =
                self.selected_entry.as_ref().or(selected_path.as_ref()) == Some(&entry.path);
            let target = Target {
                path: entry.path.clone(),
                kind: EntryKind::File,
            };
            tree = tree.push(DragArea::new(
                container(
                    row![
                        self.tree_indent(depth + 1),
                        Space::new().width(16),
                        look::icon(Icon::FileText),
                        text(&entry.name).size(13).width(Fill)
                    ]
                    .spacing(7)
                    .align_y(iced::Center),
                )
                .padding(iced::Padding {
                    left: 0.0,
                    right: 10.0,
                    top: 5.0,
                    bottom: 5.0,
                })
                .width(Fill)
                .style(move |_| {
                    look::tree_row(
                        selected,
                        self.hovered_target
                            .as_ref()
                            .is_some_and(|target| target.path == entry.path),
                    )
                }),
                self.drag_interaction(&target),
                move |phase, position| Message::PointerEntry(target.clone(), phase, position),
            ));
        }
        let scrolling_tree = scrollable(
            tree.width(
                self.explorer
                    .visible()
                    .iter()
                    .map(|(depth, _)| 180.0 + (*depth + 1) as f32 * 16.0)
                    .fold(
                        if self.viewport_width < 600.0 {
                            self.viewport_width - 38.0
                        } else {
                            f32::from(self.settings.workspace.sidebar_width)
                        },
                        f32::max,
                    ),
            ),
        )
        .direction(scrollable::Direction::Both {
            vertical: scrollable::Scrollbar::default(),
            horizontal: scrollable::Scrollbar::default(),
        })
        .height(Fill);
        let tree: Element<'_, Message> = if let Some(root) = &self.explorer.root {
            let target = Target {
                path: root.clone(),
                kind: EntryKind::Folder,
            };
            let active = self.drag.as_ref().is_some_and(|drag| {
                drag.moving
                    && drag.accepts(&target, root)
                    && self
                        .hovered_target
                        .as_ref()
                        .is_some_and(|hovered| hovered.path == *root)
            });
            let interaction = if active {
                iced::mouse::Interaction::Grabbing
            } else {
                iced::mouse::Interaction::None
            };
            stack![
                DragArea::new(
                    container(Space::new().width(Fill).height(Fill)).style(move |_| {
                        iced::widget::container::Style {
                            border: iced::Border {
                                color: look::DROP,
                                width: if active { 1.0 } else { 0.0 },
                                radius: 0.0.into(),
                            },
                            ..Default::default()
                        }
                    }),
                    interaction,
                    Message::RootDrop
                ),
                scrolling_tree
            ]
            .height(Fill)
            .width(Fill)
            .into()
        } else {
            scrolling_tree.into()
        };
        panel.push(tree).height(Fill).into()
    }

    fn tree_indent(&self, depth: usize) -> Element<'_, Message> {
        let mut guides = row![Space::new().width(8)];
        for _ in 0..depth {
            guides = guides.push(
                container(
                    container(Space::new().width(1).height(24))
                        .style(|_| look::surface(look::LINE)),
                )
                .width(16)
                .align_x(iced::Center),
            );
        }
        guides.into()
    }

    fn folder_row(&self, path: &Path, name: String, depth: usize) -> Element<'_, Message> {
        let target = Target {
            path: path.to_owned(),
            kind: EntryKind::Folder,
        };
        let expanded = self.explorer.expanded.contains(path);
        let over = self.hovered_folder.as_deref() == Some(path);
        let hovered = over || self.viewport_width < 600.0;
        let dropping = self.drag.as_ref().is_some_and(|drag| {
            drag.moving
                && self.hovered_target.as_ref().is_some_and(|target| {
                    target.path == path
                        && self
                            .explorer
                            .root
                            .as_ref()
                            .is_some_and(|root| drag.accepts(target, root))
                })
        });
        let selected = self.selected_entry.as_deref() == Some(path) || dropping;
        let label = container(
            row![
                look::icon(if expanded {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                }),
                look::icon(Icon::Folder),
                text(name)
                    .size(13)
                    .width(Fill)
                    .wrapping(iced::widget::text::Wrapping::None)
            ]
            .spacing(6)
            .align_y(iced::Center),
        )
        .width(Fill)
        .padding([5, 4]);
        let tools: Element<'_, Message> =
            if hovered && !self.drag.as_ref().is_some_and(|drag| drag.moving) {
                row![
                    self.tool(
                        Icon::FilePlus,
                        "New file",
                        Message::BeginEntry(path.to_owned(), EntryKind::File),
                        !self.busy
                    ),
                    self.tool(
                        Icon::FolderPlus,
                        "New folder",
                        Message::BeginEntry(path.to_owned(), EntryKind::Folder),
                        !self.busy
                    )
                ]
                .spacing(2)
                .into()
            } else {
                Space::new().width(0).height(28).into()
            };
        let content = container(row![self.tree_indent(depth), label, tools].align_y(iced::Center))
            .width(Fill)
            .height(32)
            .padding(iced::Padding {
                left: 0.0,
                right: 8.0,
                top: 0.0,
                bottom: 0.0,
            })
            .style(move |_| {
                let mut style = look::tree_row(selected, over);
                if dropping {
                    style.border = iced::Border {
                        color: look::DROP,
                        width: 1.0,
                        radius: 4.0.into(),
                    };
                }
                style
            });
        DragArea::new(
            content,
            self.drag_interaction(&target),
            move |phase, position| Message::PointerEntry(target.clone(), phase, position),
        )
        .into()
    }

    fn drag_interaction(&self, target: &Target) -> iced::mouse::Interaction {
        if let Some(drag) = &self.drag
            && drag.moving
        {
            if self
                .explorer
                .root
                .as_ref()
                .is_some_and(|root| drag.accepts(target, root))
            {
                iced::mouse::Interaction::Grabbing
            } else {
                iced::mouse::Interaction::NotAllowed
            }
        } else {
            iced::mouse::Interaction::Grab
        }
    }

    fn entry_input(&self, depth: usize) -> Element<'_, Message> {
        let draft = self.entry_draft.as_ref().unwrap();
        row![
            self.tree_indent(depth),
            look::icon(if draft.kind == EntryKind::File {
                Icon::FileText
            } else {
                Icon::Folder
            }),
            text_input("Name", &draft.name)
                .id("entry-name")
                .size(13)
                .padding(5)
                .on_input(Message::EntryName)
                .on_submit(Message::SubmitEntry)
                .width(Fill),
            self.tool(
                Icon::Check,
                if draft.source.is_some() {
                    "Rename"
                } else {
                    "Create"
                },
                Message::SubmitEntry,
                !self.busy && !draft.name.trim().is_empty()
            ),
            self.tool(Icon::X, "Cancel", Message::CancelEntry, !self.busy),
        ]
        .spacing(3)
        .align_y(iced::Center)
        .padding(iced::Padding {
            left: 0.0,
            right: 8.0,
            top: 3.0,
            bottom: 3.0,
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_reader(app: &mut Enote, root: &Path) -> BufferId {
        let path = root.join("book.pdf");
        std::fs::write(&path, include_bytes!("../tests/fixtures/reader.pdf")).unwrap();
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&path))));
        let id = app.active.unwrap();
        finish_test_render(app, id);
        id
    }

    fn finish_test_render(app: &mut Enote, id: BufferId) {
        let viewer = &app.readers[&id];
        let request = viewer.request;
        let result = enote::reader::render(
            include_bytes!("../tests/fixtures/reader.pdf"),
            "pdf",
            viewer.position.page,
            viewer.pixel_width,
        );
        let _ = app.update(Message::ReaderRendered(id, request, result));
        let _ = app.update(Message::ReaderRestored(id, request));
    }

    #[test]
    fn document_buffer_restores_page_zoom_and_scroll_and_stays_read_only() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        assert!(!app.editors.contains_key(&id));
        assert!(app.buffers.entries[&id].read_only);
        assert_eq!(app.readers[&id].count, Some(3));
        let _ = app.update(Message::Reader(id, ReaderAction::Next));
        finish_test_render(&mut app, id);
        let _ = app.update(Message::Reader(id, ReaderAction::ZoomIn));
        finish_test_render(&mut app, id);
        let request = app.readers[&id].request;
        let _ = app.update(Message::Reader(
            id,
            ReaderAction::Scroll {
                request,
                x: 0.2,
                y: 0.7,
            },
        ));
        let position = app.readers[&id].position;
        app.reader_store.as_ref().unwrap().flush().unwrap();
        let _ = app.update(Message::Close(CloseTarget::Buffer(id)));
        let id = open_test_reader(&mut app, &root);
        assert_eq!(app.readers[&id].position, position);
        assert_eq!(position.page, 1);
        assert_eq!(position.zoom, 1.25);
    }

    #[test]
    fn splits_keep_buffers_visible_and_closing_pane_preserves_unsaved_edits() {
        let (_dir, mut app, root) = file_fixture();
        let pdf = open_test_reader(&mut app, &root);
        let first = app.focused_pane;
        let _ = app.update(Message::Split(pane_grid::Axis::Vertical));
        let second = app.focused_pane;
        assert_ne!(first, second);
        assert_eq!(*app.panes.get(first).unwrap(), Some(pdf));
        let note = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&note))));
        let text = app.active.unwrap();
        assert_eq!(*app.panes.get(second).unwrap(), Some(text));
        app.buffers
            .entries
            .get_mut(&text)
            .unwrap()
            .set_text("unsaved split edit".into());
        let _ = app.update(Message::ClosePane(second));
        assert_eq!(app.panes.len(), 1);
        assert_eq!(app.active, Some(pdf));
        assert!(app.buffers.entries[&text].modified());
        let _ = app.update(Message::Activate(text));
        assert_eq!(app.active, Some(text));
        assert_eq!(
            app.buffers.entries[&text].text(),
            Some("unsaved split edit")
        );
    }

    fn open_two_tab_fixture(app: &mut Enote, root: &Path) -> (BufferId, BufferId) {
        let _ = app.update(Message::Opened(Some(LoadedFile::read(
            &root.join("A/nested/note.mde"),
        ))));
        let first = app.active.unwrap();
        let other = root.join("other.mde");
        std::fs::write(&other, "Other tab").unwrap();
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&other))));
        (first, app.active.unwrap())
    }

    #[test]
    fn tabs_reorder_from_pointer_release_and_restore_the_new_order() {
        let (_dir, mut app, root) = file_fixture();
        let (first, second) = open_two_tab_fixture(&mut app, &root);
        let start = iced::Point::new(20.0, 15.0);
        let bounds =
            iced::Rectangle::new(iced::Point::new(120.0, 0.0), iced::Size::new(100.0, 30.0));
        let _ = app.update(Message::TabPointer(first, Phase::Press, start, bounds));
        // No intermediate move message: a batched release must still be a drag.
        let _ = app.update(Message::TabPointer(
            second,
            Phase::Release,
            iced::Point::new(210.0, 15.0),
            bounds,
        ));
        assert_eq!(app.tab_order, vec![second, first]);
        assert!(app.tab_drag.is_none());
        let snapshot = app.workspace_snapshot();
        snapshot.save(&root).unwrap();
        app.apply_volume(OpenedVolume::open(&root, &app.base_settings).unwrap())
            .unwrap();
        assert_eq!(app.workspace_snapshot(), snapshot);
        let _ = app.update(Message::NextTab);
        assert_eq!(app.buffers.entries[&app.active.unwrap()].name(), "note.mde");
    }

    #[test]
    fn dragging_a_hidden_tab_to_a_pane_edge_creates_a_split_and_preserves_edits() {
        let (_dir, mut app, root) = file_fixture();
        let (first, second) = open_two_tab_fixture(&mut app, &root);
        app.buffers
            .entries
            .get_mut(&first)
            .unwrap()
            .set_text("Unsaved tab content".into());
        let target = app.focused_pane;
        let bounds =
            iced::Rectangle::new(iced::Point::new(0.0, 50.0), iced::Size::new(600.0, 500.0));
        let _ = app.update(Message::TabPointer(
            first,
            Phase::Press,
            iced::Point::new(20.0, 15.0),
            bounds,
        ));
        let _ = app.update(Message::PanePointer(
            target,
            Phase::Release,
            iced::Point::new(590.0, 250.0),
            bounds,
        ));
        assert_eq!(app.panes.len(), 2);
        assert_eq!(*app.panes.get(target).unwrap(), Some(second));
        assert_eq!(*app.panes.get(app.focused_pane).unwrap(), Some(first));
        assert_eq!(app.active, Some(first));
        assert_eq!(
            app.buffers.entries[&first].text(),
            Some("Unsaved tab content")
        );
        assert!(app.buffers.entries[&first].modified());
    }

    #[test]
    fn dropping_visible_tab_in_another_pane_moves_it_without_duplicating_buffers() {
        let (_dir, mut app, root) = file_fixture();
        let (first, second) = open_two_tab_fixture(&mut app, &root);
        let original = app.focused_pane;
        let _ = app.update(Message::Split(pane_grid::Axis::Vertical));
        let target = app.focused_pane;
        assert_eq!(*app.panes.get(target).unwrap(), Some(first));
        let bounds =
            iced::Rectangle::new(iced::Point::new(0.0, 50.0), iced::Size::new(600.0, 500.0));
        let _ = app.update(Message::TabPointer(
            second,
            Phase::Press,
            iced::Point::new(20.0, 15.0),
            bounds,
        ));
        let _ = app.update(Message::PanePointer(
            target,
            Phase::Release,
            bounds.center(),
            bounds,
        ));
        assert_eq!(*app.panes.get(target).unwrap(), Some(second));
        assert_eq!(*app.panes.get(original).unwrap(), Some(first));
        assert_eq!(app.buffers.entries.len(), 2);
    }

    #[test]
    fn tab_click_activates_and_release_outside_targets_cancels_drag() {
        let (_dir, mut app, root) = file_fixture();
        let (first, second) = open_two_tab_fixture(&mut app, &root);
        let point = iced::Point::new(20.0, 15.0);
        let bounds = iced::Rectangle::new(iced::Point::ORIGIN, iced::Size::new(100.0, 30.0));
        let _ = app.update(Message::TabPointer(first, Phase::Press, point, bounds));
        let _ = app.update(Message::TabPointer(first, Phase::Release, point, bounds));
        assert_eq!(app.active, Some(first));
        let _ = app.update(Message::TabPointer(second, Phase::Press, point, bounds));
        let _ = app.update(Message::CursorMoved(iced::Point::new(600.0, 600.0)));
        let _ = app.update(Message::MouseUp);
        let _ = app.update(Message::EndTabDrag(app.tab_drag_serial));
        assert!(app.tab_drag.is_none());
        assert_eq!(app.tab_order, vec![first, second]);
        assert_eq!(app.panes.len(), 1);
    }

    #[test]
    fn dragging_panes_swaps_and_repositions_buffers_without_losing_edits() {
        let (_dir, mut app, root) = file_fixture();
        let note = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&note))));
        let first_id = app.active.unwrap();
        let first_pane = app.focused_pane;
        edit(&mut app, 'x');
        let unsaved = app.buffers.entries[&first_id].text().unwrap().to_owned();
        let _ = app.update(Message::Split(pane_grid::Axis::Vertical));
        let second_pane = app.focused_pane;
        let other = root.join("second.mde");
        std::fs::write(&other, "Second buffer").unwrap();
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&other))));
        let second_id = app.active.unwrap();
        let _ = app.update(Message::DragPane(pane_grid::DragEvent::Dropped {
            pane: first_pane,
            target: pane_grid::Target::Pane(second_pane, pane_grid::Region::Center),
        }));
        assert_eq!(*app.panes.get(first_pane).unwrap(), Some(first_id));
        assert_eq!(*app.panes.get(second_pane).unwrap(), Some(second_id));
        assert_eq!(app.active, Some(first_id));
        assert_eq!(app.focused_pane, first_pane);
        let PaneLayout::Split { a, b, .. } = app.workspace_snapshot().layout else {
            panic!("Expected split");
        };
        assert_eq!(*a, PaneLayout::Pane { tab: Some(1) });
        assert_eq!(*b, PaneLayout::Pane { tab: Some(0) });
        let _ = app.update(Message::DragPane(pane_grid::DragEvent::Dropped {
            pane: first_pane,
            target: pane_grid::Target::Pane(
                second_pane,
                pane_grid::Region::Edge(pane_grid::Edge::Bottom),
            ),
        }));
        assert!(matches!(
            app.workspace_snapshot().layout,
            PaneLayout::Split {
                vertical: false,
                ..
            }
        ));
        assert_eq!(
            app.buffers.entries[&first_id].text(),
            Some(unsaved.as_str())
        );
        assert!(app.buffers.entries[&first_id].modified());
        let layout = app.workspace_snapshot().layout;
        let _ = app.update(Message::DragPane(pane_grid::DragEvent::Canceled {
            pane: second_pane,
        }));
        assert_eq!(app.workspace_snapshot().layout, layout);
    }

    #[test]
    fn volume_restores_tabs_active_buffer_layout_and_reader_position() {
        let (_dir, mut app, root) = file_fixture();
        let pdf = open_test_reader(&mut app, &root);
        let _ = app.update(Message::Reader(pdf, ReaderAction::Next));
        finish_test_render(&mut app, pdf);
        app.reader_store.as_ref().unwrap().flush().unwrap();
        let _ = app.update(Message::Split(pane_grid::Axis::Vertical));
        let note = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&note))));
        let snapshot = app.workspace_snapshot();
        snapshot.save(&root).unwrap();
        let opened = OpenedVolume::open(&root, &app.base_settings).unwrap();
        let mut restored = Enote::default();
        restored.apply_volume(opened).unwrap();
        assert_eq!(restored.workspace_snapshot(), snapshot);
        assert_eq!(
            restored.buffers.entries[&restored.active.unwrap()].name(),
            "note.mde"
        );
        assert_eq!(restored.panes.len(), 2);
        let pdf = restored.readers.keys().next().copied().unwrap();
        assert_eq!(restored.readers[&pdf].position.page, 1);
        let _ = restored.update(Message::Close(CloseTarget::Buffer(
            restored.active.unwrap(),
        )));
        restored.workspace_snapshot().save(&root).unwrap();
        restored
            .apply_volume(OpenedVolume::open(&root, &restored.base_settings).unwrap())
            .unwrap();
        assert_eq!(restored.buffers.entries.len(), 1);
        assert!(restored.editors.is_empty());
    }

    #[test]
    fn startup_restores_window_geometry_and_the_saved_volume_workspace() {
        let (dir, mut app, root) = file_fixture();
        let path = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&path))));
        let snapshot = app.workspace_snapshot();
        snapshot.save(&root).unwrap();
        let paths = enote::settings::SettingsPaths {
            global: dir.path().join("global.json"),
            user: dir.path().join("config/settings.json"),
        };
        let session = SessionStore::new(&paths);
        session.remember(&root).unwrap();
        session
            .remember_window(&WindowPlacement {
                width: 1210.0,
                height: 810.0,
                position: Some([42.0, 63.0]),
            })
            .unwrap();
        let restored = Enote::from_startup(Startup::load(paths));
        assert_eq!(restored.workspace_snapshot(), snapshot);
        assert_eq!(
            (restored.viewport_width, restored.viewport_height),
            (1210.0, 810.0)
        );
        assert_eq!(restored.window_position, Some([42.0, 63.0]));
    }

    #[test]
    fn restored_hidden_document_is_rendered_only_when_activated() {
        let (_dir, mut app, root) = file_fixture();
        let first = open_test_reader(&mut app, &root);
        let second_path = root.join("second.pdf");
        std::fs::write(&second_path, include_bytes!("../tests/fixtures/reader.pdf")).unwrap();
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&second_path))));
        app.workspace_snapshot().save(&root).unwrap();
        app.apply_volume(OpenedVolume::open(&root, &app.base_settings).unwrap())
            .unwrap();
        let _ = app.update(Message::RestoreReaders);
        assert!(app.readers[&app.active.unwrap()].worker.is_some());
        assert!(app.readers[&first].worker.is_none());
        let _ = app.update(Message::Activate(first));
        assert!(app.readers[&first].worker.is_some());
    }

    #[test]
    fn changing_volumes_keeps_current_window_geometry() {
        let (dir, mut app, _root) = file_fixture();
        let _ = app.update(Message::Viewport(iced::Size::new(1320.0, 840.0)));
        let _ = app.update(Message::WindowMoved(iced::Point::new(-200.0, 100.0)));
        let other = OpenedVolume::create(dir.path(), "Other volume", &app.base_settings).unwrap();
        app.apply_volume(other).unwrap();
        assert_eq!((app.viewport_width, app.viewport_height), (1320.0, 840.0));
        assert_eq!(app.window_position, Some([-200.0, 100.0]));
    }

    #[test]
    fn resize_retains_rendered_pages_and_continuous_scroll_tracks_page() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        let request = app.readers[&id].request;
        let slot = app.readers[&id].slot_height();
        let _ = app.update(Message::Reader(
            id,
            ReaderAction::StreamScroll {
                request,
                x: 0.0,
                y: slot * 1.4,
            },
        ));
        assert_eq!(app.readers[&id].position.page, 1);
        assert!((app.readers[&id].position.y - 0.4).abs() < 0.001);
        assert!(app.readers[&id].image.is_some());
        finish_test_render(&mut app, id);
        let _ = app.update(Message::Viewport(iced::Size::new(1200.0, 750.0)));
        assert!(app.readers[&id].loading);
        assert!(app.readers[&id].image.is_some());
        assert!(app.readers[&id].pages.contains_key(&1));
        let request = app.readers[&id].request;
        let _ = app.update(Message::Reader(
            id,
            ReaderAction::StreamScroll {
                request,
                x: 0.0,
                y: 0.0,
            },
        ));
        assert_eq!(app.readers[&id].position.page, 1);
        finish_test_render(&mut app, id);
        assert!(!app.readers[&id].loading);
    }

    #[test]
    fn prefetched_page_navigation_reuses_the_existing_image_without_loading() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        let request = app.readers[&id].request;
        let width = app.readers[&id].pixel_width;
        let page = enote::reader::render(
            include_bytes!("../tests/fixtures/reader.pdf"),
            "pdf",
            1,
            width,
        )
        .unwrap();
        let _ = app.update(Message::ReaderPrefetched(id, request, Ok(page)));
        let handle = app.readers[&id].pages[&1].handle.clone();
        let _ = app.update(Message::Reader(id, ReaderAction::Next));
        assert!(!app.readers[&id].loading);
        finish_test_render(&mut app, id);
        assert_eq!(app.readers[&id].image.as_ref(), Some(&handle));
        assert!(app.readers[&id].pages.contains_key(&0));
    }

    #[test]
    fn fast_scroll_updates_the_target_even_when_previous_page_is_loading() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        for page in [1, 2] {
            let viewer = &app.readers[&id];
            let (request, slot) = (viewer.request, viewer.slot_height());
            let _ = app.update(Message::Reader(
                id,
                ReaderAction::StreamScroll {
                    request,
                    x: 0.0,
                    y: slot * (page as f32 + 0.2),
                },
            ));
            assert_eq!(app.readers[&id].position.page, page);
        }
        finish_test_render(&mut app, id);
        assert!(!app.readers[&id].loading);
        assert!(app.readers[&id].pages.contains_key(&2));
    }

    #[test]
    fn sidebar_drag_clamps_width_and_releases_capture() {
        let mut app = Enote::default();
        let _ = app.update(Message::ResizeSidebar(
            Phase::Press,
            iced::Point::new(600.0, 100.0),
        ));
        let _ = app.update(Message::CursorMoved(iced::Point::new(950.0, 100.0)));
        assert_eq!(app.settings.workspace.sidebar_width, 180);
        let _ = app.update(Message::MouseUp);
        assert!(!app.resizing_sidebar);
    }

    #[test]
    fn multiple_external_drops_are_queued_and_keep_import_errors_visible() {
        let (_dir, mut app, root) = file_fixture();
        let external = tempfile::tempdir().unwrap();
        let first = external.path().join("one.pdf");
        let second = external.path().join("two.pdf");
        std::fs::write(&first, include_bytes!("../tests/fixtures/reader.pdf")).unwrap();
        std::fs::write(&second, include_bytes!("../tests/fixtures/reader.pdf")).unwrap();
        let _ = app.update(Message::FileDropped(first));
        let _ = app.update(Message::FileDropped(second.clone()));
        assert!(app.busy && app.importing);
        assert_eq!(app.pending_imports.len(), 1);
        let _ = app.update(Message::FileCompleted(Err("first import failed".into())));
        assert!(app.busy && app.importing);
        assert!(app.pending_imports.is_empty());
        let result = FileOperation::Import {
            source: second.clone(),
            destination: root.clone(),
        }
        .run(app.volume.as_ref().unwrap());
        let _ = app.update(Message::FileCompleted(result));
        assert!(!app.busy && !app.importing);
        assert_eq!(app.status, "first import failed");
        assert!(second.exists());
        let _ = app.update(Message::Opened(Some(LoadedFile::read(
            &root.join("two.pdf"),
        ))));
        let id = app.active.unwrap();
        finish_test_render(&mut app, id);
        assert!(app.readers[&id].image.is_some());
    }

    #[test]
    fn pdf_scroll_without_horizontal_overflow_remains_persistable() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        let request = app.readers[&id].request;
        let _ = app.update(Message::Reader(
            id,
            ReaderAction::Scroll {
                request,
                x: f32::NAN,
                y: 0.6,
            },
        ));
        let store = app.reader_store.as_ref().unwrap();
        store.flush().unwrap();
        let position = store.position(&root.join("book.pdf")).unwrap();
        assert_eq!(position.x, 0.0);
        assert_eq!(position.y, 0.6);
        assert!(app.status.is_empty());
    }

    #[test]
    fn open_pdf_moves_to_folder_and_back_to_root_with_reader_intact() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        let _ = app.update(Message::Reader(id, ReaderAction::Next));
        finish_test_render(&mut app, id);
        for (source, destination) in [
            (root.join("book.pdf"), root.join("B")),
            (root.join("B/book.pdf"), root.clone()),
        ] {
            let _ = app.update(Message::PointerEntry(
                Target {
                    path: source.clone(),
                    kind: EntryKind::File,
                },
                Phase::Press,
                iced::Point::new(800.0, 200.0),
            ));
            let _ = app.update(Message::PointerEntry(
                Target {
                    path: destination.clone(),
                    kind: EntryKind::Folder,
                },
                Phase::Release,
                iced::Point::new(800.0, 100.0),
            ));
            assert!(app.busy);
            let result = FileOperation::Move {
                source,
                destination: destination.clone(),
            }
            .run(app.volume.as_ref().unwrap());
            let _ = app.update(Message::FileCompleted(result));
            assert!(!app.busy);
            assert_eq!(app.active, Some(id));
            assert!(app.readers[&id].image.is_some());
            assert_eq!(app.readers[&id].position.page, 1);
            assert_eq!(
                app.reader_store
                    .as_ref()
                    .unwrap()
                    .position(&destination.join("book.pdf"))
                    .unwrap()
                    .page,
                1
            );
        }
    }

    #[test]
    fn disabling_remember_starts_at_first_page_and_does_not_write_positions() {
        let (_dir, mut app, root) = file_fixture();
        let store = app.reader_store.as_ref().unwrap();
        store
            .update(
                &root.join("book.pdf"),
                enote::reader::ReadingPosition {
                    page: 2,
                    zoom: 2.0,
                    x: 0.0,
                    y: 0.8,
                },
            )
            .unwrap();
        store.flush().unwrap();
        let path = root.join(".enote/viewer-state.json");
        let before = std::fs::read(&path).unwrap();
        app.settings.viewer.remember_position = false;
        let id = open_test_reader(&mut app, &root);
        assert_eq!(app.readers[&id].position, Default::default());
        let _ = app.update(Message::Reader(id, ReaderAction::Next));
        finish_test_render(&mut app, id);
        app.reader_store.as_ref().unwrap().flush().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn closed_document_does_not_accept_stale_render_or_scroll_results() {
        let (_dir, mut app, root) = file_fixture();
        let id = open_test_reader(&mut app, &root);
        let old = app.readers[&id].request;
        let _ = app.update(Message::Reader(id, ReaderAction::Next));
        let result = enote::reader::render(
            include_bytes!("../tests/fixtures/reader.pdf"),
            "pdf",
            0,
            600.0,
        );
        let _ = app.update(Message::ReaderRendered(id, old, result));
        assert!(app.readers[&id].loading);
        assert_eq!(app.readers[&id].position.page, 1);
        let _ = app.update(Message::Close(CloseTarget::Buffer(id)));
        let _ = app.update(Message::Reader(
            id,
            ReaderAction::Scroll {
                request: old,
                x: 0.0,
                y: 1.0,
            },
        ));
        assert!(!app.readers.contains_key(&id));
        assert_eq!(app.shortcut("pagedown", keyboard::Modifiers::empty()), None);
    }

    fn file_fixture() -> (tempfile::TempDir, Enote, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Enote::default();
        let opened = OpenedVolume::create(dir.path(), "Notes", &app.base_settings).unwrap();
        let root = opened.storage.root.clone();
        std::fs::create_dir_all(root.join("A/nested")).unwrap();
        std::fs::create_dir(root.join("B")).unwrap();
        std::fs::write(root.join("A/nested/note.mde"), "saved").unwrap();
        app.apply_volume(OpenedVolume::open(&root, &app.base_settings).unwrap())
            .unwrap();
        (dir, app, root)
    }

    #[test]
    fn row_press_uses_its_own_path_and_drag_does_not_toggle_destination() {
        let (_dir, mut app, root) = file_fixture();
        let _ = app.update(Message::HoverEntry(root.join("B"), EntryKind::Folder));
        let _ = app.update(Message::CursorMoved(iced::Point::new(800.0, 100.0)));
        let _ = app.update(Message::ArmEntry(root.join("A"), EntryKind::Folder));
        assert_eq!(app.drag.as_ref().unwrap().source.path, root.join("A"));
        let _ = app.update(Message::CursorMoved(iced::Point::new(800.0, 140.0)));
        let _ = app.update(Message::HoverFolder(root.join("B"), true));
        let _ = app.update(Message::PointerEntry(
            Target {
                path: root.join("B"),
                kind: EntryKind::Folder,
            },
            Phase::Release,
            iced::Point::new(800.0, 140.0),
        ));
        assert!(app.busy);
        assert!(app.drag.is_none());
        // Tasks are executed by iced; the model must not toggle the target on release.
        assert!(!app.explorer.expanded.contains(&root.join("B")));
    }

    #[test]
    fn moving_to_unvisited_deep_folder_reveals_all_ancestors() {
        let (_dir, mut app, root) = file_fixture();
        std::fs::create_dir_all(root.join("B/deep/target")).unwrap();
        let outcome = FileOperation::Move {
            source: root.join("A"),
            destination: root.join("B/deep/target"),
        }
        .run(app.volume.as_ref().unwrap())
        .unwrap();
        app.apply_file_outcome(outcome);
        for path in [
            root.join("B"),
            root.join("B/deep"),
            root.join("B/deep/target"),
        ] {
            assert!(app.explorer.expanded.contains(&path));
        }
        assert!(
            app.explorer
                .visible()
                .iter()
                .any(|(_, entry)| entry.path == root.join("B/deep/target/A"))
        );
    }

    #[test]
    fn drop_uses_release_row_even_when_hover_is_stale() {
        let (_dir, mut app, root) = file_fixture();
        let source = Target {
            path: root.join("A/nested/note.mde"),
            kind: EntryKind::File,
        };
        let _ = app.update(Message::PointerEntry(
            source,
            Phase::Press,
            iced::Point::new(800.0, 100.0),
        ));
        let _ = app.update(Message::HoverFolder(root.join("A"), true));
        let _ = app.update(Message::PointerEntry(
            Target {
                path: root.join("B"),
                kind: EntryKind::Folder,
            },
            Phase::Release,
            iced::Point::new(800.0, 160.0),
        ));
        assert!(app.busy);
        assert!(app.drag.is_none());
        assert!(!app.explorer.expanded.contains(&root.join("B")));
    }

    #[test]
    fn dropping_outside_or_pressing_escape_cancels_without_moving() {
        let (_dir, mut app, root) = file_fixture();
        for escape in [false, true] {
            let _ = app.update(Message::PointerEntry(
                Target {
                    path: root.join("A"),
                    kind: EntryKind::Folder,
                },
                Phase::Press,
                iced::Point::new(800.0, 100.0),
            ));
            let _ = app.update(Message::CursorMoved(iced::Point::new(800.0, 160.0)));
            let _ = app.update(Message::HoverFolder(root.join("B"), true));
            if escape {
                let _ = app.update(Message::KeyPressed(
                    "Escape".into(),
                    keyboard::Modifiers::empty(),
                ));
            } else {
                let _ = app.update(Message::MouseUp);
            }
            assert!(app.drag.is_none());
            assert!(!app.busy);
            assert!(root.join("A/nested/note.mde").exists());
        }
    }

    #[test]
    fn invalid_drop_does_not_move_open_or_toggle_objects() {
        let (_dir, mut app, root) = file_fixture();
        for path in [
            root.join("A"),
            root.join("A/nested"),
            root.join("A/nested/note.mde"),
            root.clone(),
        ] {
            let source = Target {
                path: root.join("A"),
                kind: EntryKind::Folder,
            };
            let _ = app.update(Message::PointerEntry(
                source,
                Phase::Press,
                iced::Point::new(800.0, 100.0),
            ));
            let target = Target {
                kind: if path.is_dir() {
                    EntryKind::Folder
                } else {
                    EntryKind::File
                },
                path,
            };
            let _ = app.update(Message::PointerEntry(
                target,
                Phase::Release,
                iced::Point::new(800.0, 160.0),
            ));
            assert!(!app.busy);
            assert!(app.drag.is_none());
            assert!(app.active.is_none());
            assert_eq!(app.explorer.expanded.len(), 1);
        }
    }

    #[test]
    fn row_click_without_movement_still_expands_folder() {
        let (_dir, mut app, root) = file_fixture();
        let target = Target {
            path: root.join("A"),
            kind: EntryKind::Folder,
        };
        let position = iced::Point::new(800.0, 100.0);
        let _ = app.update(Message::PointerEntry(
            target.clone(),
            Phase::Press,
            position,
        ));
        let _ = app.update(Message::PointerEntry(target, Phase::Release, position));
        assert!(app.explorer.expanded.contains(&root.join("A")));
        assert!(app.drag.is_none());
    }

    #[test]
    fn blank_panel_drop_moves_nested_file_and_folder_to_root() {
        for folder in [false, true] {
            let (_dir, mut app, root) = file_fixture();
            let old = root.join("A/nested/note.mde");
            let _ = app.update(Message::Opened(Some(LoadedFile::read(&old))));
            let id = app.active.unwrap();
            edit(&mut app, 'x');
            let source = if folder {
                root.join("A/nested")
            } else {
                old.clone()
            };
            let kind = if folder {
                EntryKind::Folder
            } else {
                EntryKind::File
            };
            let _ = app.update(Message::PointerEntry(
                Target {
                    path: source.clone(),
                    kind,
                },
                Phase::Press,
                iced::Point::new(800.0, 100.0),
            ));
            let _ = app.update(Message::RootDrop(
                Phase::Move,
                iced::Point::new(800.0, 400.0),
            ));
            assert_eq!(app.hovered_target.as_ref().unwrap().path, root);
            let _ = app.update(Message::RootDrop(
                Phase::Release,
                iced::Point::new(800.0, 400.0),
            ));
            assert!(app.busy);
            assert!(app.drag.is_none());
            let outcome = FileOperation::Move {
                source,
                destination: root.clone(),
            }
            .run(app.volume.as_ref().unwrap())
            .unwrap();
            let _ = app.update(Message::FileCompleted(Ok(outcome)));
            assert!(!old.exists());
            assert!(app.buffers.entries[&id].modified());
            let new = if folder {
                root.join("nested/note.mde")
            } else {
                root.join("note.mde")
            };
            assert_eq!(
                app.buffers
                    .path(app.buffers.entries[&id].source.as_ref().unwrap())
                    .unwrap(),
                new
            );
        }
    }

    #[test]
    fn blank_panel_click_and_root_entry_drop_are_no_ops() {
        let (_dir, mut app, root) = file_fixture();
        let position = iced::Point::new(800.0, 400.0);
        let _ = app.update(Message::RootDrop(Phase::Press, position));
        let _ = app.update(Message::RootDrop(Phase::Release, position));
        assert!(app.explorer.expanded.contains(&root));
        assert!(app.drag.is_none());
        let _ = app.update(Message::PointerEntry(
            Target {
                path: root.join("A"),
                kind: EntryKind::Folder,
            },
            Phase::Press,
            iced::Point::new(800.0, 100.0),
        ));
        let _ = app.update(Message::RootDrop(Phase::Release, position));
        assert!(!app.busy);
        assert!(root.join("A").is_dir());
    }

    #[test]
    fn moving_dirty_open_folder_preserves_editor_and_saves_to_new_path() {
        let (_dir, mut app, root) = file_fixture();
        let old = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&old))));
        let id = app.active.unwrap();
        edit(&mut app, 'x');
        let text = app.editors[&id].text();
        let cursor = app.editors[&id].cursor();
        let outcome = FileOperation::Move {
            source: root.join("A"),
            destination: root.join("B"),
        }
        .run(app.volume.as_ref().unwrap())
        .unwrap();
        let _ = app.update(Message::FileCompleted(Ok(outcome)));
        assert_eq!(app.active, Some(id));
        assert_eq!(app.editors[&id].cursor(), cursor);
        assert_eq!(app.editors[&id].text(), text);
        assert!(app.buffers.entries[&id].modified());
        let source = app.buffers.entries[&id].source.as_ref().unwrap();
        assert_eq!(
            app.buffers.path(source).unwrap(),
            root.join("B/A/nested/note.mde")
        );
        let _ = app.update(Message::Undo);
        assert_eq!(app.buffers.entries[&id].text(), Some("saved"));
        let _ = app.update(Message::Redo);
        let saved = app.buffers.save_job(id, None).unwrap().write().unwrap();
        app.buffers.saved(saved).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("B/A/nested/note.mde")).unwrap(),
            text
        );
        assert!(!old.exists());
    }

    #[test]
    fn context_delete_warns_about_dirty_descendants_and_can_be_cancelled_or_restored() {
        let (_dir, mut app, root) = file_fixture();
        let path = root.join("A/nested/note.mde");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&path))));
        let id = app.active.unwrap();
        edit(&mut app, 'x');
        let _ = app.update(Message::ContextRequested(root.join("A"), EntryKind::Folder));
        assert!(app.context_menu.is_some());
        let _ = app.update(Message::ContextAction(ContextAction::Delete));
        assert!(matches!(
            app.file_prompt,
            Some(FilePrompt::Delete { modified: true, .. })
        ));
        let _ = app.update(Message::CancelFilePrompt);
        assert!(path.exists());
        assert!(app.buffers.entries[&id].modified());
        let outcome = FileOperation::Trash(root.join("A"))
            .run(app.volume.as_ref().unwrap())
            .unwrap();
        let _ = app.update(Message::FileCompleted(Ok(outcome)));
        assert!(app.active.is_none());
        assert!(app.buffers.entries.is_empty());
        let outcome = FileOperation::Restore(app.last_deleted.clone().unwrap())
            .run(app.volume.as_ref().unwrap())
            .unwrap();
        let _ = app.update(Message::FileCompleted(Ok(outcome)));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved");
        assert!(app.last_deleted.is_none());
    }

    #[test]
    fn context_menu_and_move_dialog_cancel_without_mutation() {
        let (_dir, mut app, root) = file_fixture();
        let path = root.join("A");
        let _ = app.update(Message::ContextRequested(path.clone(), EntryKind::Folder));
        let _ = app.update(Message::KeyPressed(
            "Escape".into(),
            keyboard::Modifiers::empty(),
        ));
        assert!(app.context_menu.is_none());
        let _ = app.update(Message::ContextRequested(path.clone(), EntryKind::Folder));
        let _ = app.update(Message::ContextAction(ContextAction::Move));
        assert!(app.file_prompt.is_some());
        let _ = app.update(Message::KeyPressed(
            "Escape".into(),
            keyboard::Modifiers::empty(),
        ));
        assert!(app.file_prompt.is_none());
        assert!(path.exists());
    }

    fn edit(app: &mut Enote, value: char) {
        let _ = app.update(Message::Edit(
            app.active.unwrap(),
            text_editor::Action::Edit(text_editor::Edit::Insert(value)),
        ));
    }

    #[test]
    fn switching_retains_text_cursor_and_independent_history() {
        let mut app = Enote::default();
        app.new_buffer();
        let first = app.active.unwrap();
        edit(&mut app, 'a');
        let cursor = app.editors[&first].cursor();
        app.new_buffer();
        let second = app.active.unwrap();
        edit(&mut app, 'b');
        let _ = app.update(Message::Activate(first));
        assert_eq!(app.editors[&first].cursor(), cursor);
        assert_eq!(app.buffers.entries[&first].text(), Some("a"));
        let _ = app.update(Message::Undo);
        assert_eq!(app.buffers.entries[&first].text(), Some(""));
        assert!(!app.buffers.entries[&first].modified());
        assert_eq!(app.buffers.entries[&second].text(), Some("b"));
        let _ = app.update(Message::Redo);
        assert_eq!(app.editors[&first].text(), "a");
        let _ = app.update(Message::Undo);
        edit(&mut app, 'c');
        assert!(!app.buffers.entries[&first].can_redo());
    }

    #[test]
    fn closing_dirty_buffer_requires_confirmation() {
        let mut app = Enote::default();
        app.new_buffer();
        let id = app.active.unwrap();
        edit(&mut app, 'a');
        let _ = app.update(Message::Close(CloseTarget::Buffer(id)));
        assert!(app.pending_close.is_some());
        edit(&mut app, 'b');
        assert_eq!(app.buffers.entries[&id].text(), Some("a"));
        let _ = app.update(Message::Cancel);
        assert!(app.buffers.entries.contains_key(&id));
        let _ = app.update(Message::Close(CloseTarget::Window));
        assert!(matches!(app.pending_close, Some(CloseTarget::Window)));
        let _ = app.update(Message::Cancel);
        let _ = app.update(Message::Close(CloseTarget::Buffer(id)));
        let _ = app.update(Message::Discard);
        assert!(!app.buffers.entries.contains_key(&id));
        assert!(app.buffers.entries.is_empty());
        assert!(app.active.is_none());
    }

    #[test]
    fn editor_preserves_line_endings_and_final_newlines() {
        for value in ["", "a", "a\n", "a\r\n", "a\r\nb\r\n", "text\n\n"] {
            assert_eq!(
                text_editor::Content::<iced::Renderer>::with_text(value).text(),
                value
            );
        }
    }

    #[test]
    fn read_only_buffer_accepts_selection_but_rejects_edits() {
        let mut app = Enote::default();
        app.new_buffer();
        let id = app.active.unwrap();
        edit(&mut app, 'a');
        app.buffers.entries.get_mut(&id).unwrap().read_only = true;
        edit(&mut app, 'b');
        let _ = app.update(Message::Edit(id, text_editor::Action::SelectAll));
        assert_eq!(app.editors[&id].text(), "a");
        assert!(app.editors[&id].cursor().selection.is_some());
    }

    #[test]
    fn undo_clamps_multiline_and_unicode_cursor_positions() {
        let mut app = Enote::default();
        app.new_buffer();
        let id = app.active.unwrap();
        for value in ["\u{0436}", "abc\n\u{0436}\u{0436}\nlast"] {
            let _ = app.update(Message::Edit(id, text_editor::Action::SelectAll));
            let _ = app.update(Message::Edit(
                id,
                text_editor::Action::Edit(text_editor::Edit::Paste(std::sync::Arc::new(
                    value.into(),
                ))),
            ));
        }
        let _ = app.update(Message::Undo);
        edit(&mut app, 'x');
        assert_eq!(app.buffers.entries[&id].text(), Some("\u{0436}x"));
    }

    #[test]
    fn initial_workspace_is_empty_and_commands_without_buffer_are_safe() {
        let mut app = Enote::default();
        assert!(app.buffers.entries.is_empty());
        assert!(app.active.is_none());
        assert!(app.sidebar);
        for message in [Message::Save, Message::Undo, Message::Redo] {
            let _ = app.update(message);
        }
        assert!(app.active.is_none());
    }

    #[test]
    fn configured_workspace_keeps_empty_start_and_applies_preferences() {
        let mut settings = Settings::default();
        settings.window.width = 1100;
        settings.workspace.sidebar_visible = false;
        settings.workspace.sidebar_width = 300;
        settings.editor.font_size = 20;
        let app = Enote::with_settings(settings, String::new());
        assert!(app.active.is_none());
        assert!(!app.sidebar);
        assert_eq!(app.viewport_width, 1100.0);
        assert_eq!(app.settings.workspace.sidebar_width, 300);
        assert_eq!(app.settings.editor.font_size, 20);
    }

    #[test]
    fn volume_navigation_opens_files_and_retains_tree_after_last_tab_closes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "note").unwrap();
        let mut app = Enote::default();
        let opened = OpenedVolume::open(dir.path(), &app.base_settings);
        let _ = app.update(Message::VolumeOpened(Some(opened)));
        assert!(app.active.is_none());
        assert_eq!(app.explorer.visible()[0].1.name, "note.md");
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&path))));
        let id = app.active.unwrap();
        assert_eq!(app.editors[&id].text(), "note");
        let _ = app.update(Message::Close(CloseTarget::Buffer(id)));
        assert!(app.active.is_none());
        assert_eq!(app.explorer.visible().len(), 1);
    }

    #[test]
    fn opening_file_in_narrow_window_reveals_editor_and_panel_can_be_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "note").unwrap();
        let mut app = Enote::default();
        let _ = app.update(Message::Viewport(iced::Size::new(390.0, 600.0)));
        let _ = app.update(Message::Opened(Some(LoadedFile::read(&path))));
        assert!(!app.sidebar);
        assert!(app.active.is_some());
        let _ = app.update(Message::ToggleSidebar);
        assert!(app.sidebar);
    }

    #[test]
    fn switching_volume_protects_dirty_buffers_until_success_and_resets_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Enote::default();
        let first = OpenedVolume::create(dir.path(), "First", &app.base_settings).unwrap();
        std::fs::write(
            first.storage.settings_path(),
            "{\"editor\":{\"font_size\":22}}",
        )
        .unwrap();
        let first = OpenedVolume::open(&first.storage.root, &app.base_settings).unwrap();
        let second = OpenedVolume::create(dir.path(), "Second", &app.base_settings).unwrap();
        app.apply_volume(first).unwrap();
        assert_eq!(app.settings.editor.font_size, 22);
        app.new_buffer();
        let id = app.active.unwrap();
        edit(&mut app, 'a');
        let _ = app.update(Message::OpenVolume);
        assert!(matches!(
            app.pending_close,
            Some(CloseTarget::SwitchVolume(VolumeAction::Open))
        ));
        let _ = app.update(Message::Cancel);
        assert_eq!(app.buffers.entries[&id].text(), Some("a"));
        let _ = app.update(Message::OpenVolume);
        let _ = app.update(Message::Discard);
        let _ = app.update(Message::VolumeOpened(None));
        assert_eq!(app.buffers.entries[&id].text(), Some("a"));
        let _ = app.update(Message::VolumeOpened(Some(Err("cannot open".into()))));
        assert_eq!(app.buffers.entries[&id].text(), Some("a"));
        let _ = app.update(Message::OpenVolume);
        let _ = app.update(Message::Discard);
        let _ = app.update(Message::VolumeOpened(Some(Ok(second))));
        assert!(app.buffers.entries.is_empty());
        assert_eq!(app.settings.editor.font_size, 16);
        assert!(app.volume.as_ref().unwrap().root.ends_with("Second"));
    }

    #[test]
    fn creates_nested_folder_and_mde_then_edits_saves_and_reopens_in_same_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Enote::default();
        let opened = OpenedVolume::create(dir.path(), "Notes", &app.base_settings).unwrap();
        let root = opened.storage.root.clone();
        app.apply_volume(opened).unwrap();
        let _ = app.update(Message::BeginEntry(root.clone(), EntryKind::Folder));
        let _ = app.update(Message::EntryName("Research".into()));
        let created = CreatedEntry::create(
            app.volume.as_ref().unwrap(),
            app.entry_draft.as_ref().unwrap(),
        )
        .unwrap();
        let _ = app.update(Message::EntryCreated(Ok(created)));
        assert!(root.join("Research").is_dir());
        let _ = app.update(Message::New);
        assert_eq!(
            app.entry_draft.as_ref().unwrap().parent,
            root.join("Research")
        );
        let _ = app.update(Message::EntryName("Note".into()));
        let created = CreatedEntry::create(
            app.volume.as_ref().unwrap(),
            app.entry_draft.as_ref().unwrap(),
        )
        .unwrap();
        let _ = app.update(Message::EntryCreated(Ok(created)));
        let id = app.active.unwrap();
        assert_eq!(
            app.buffers.entries[&id].format(),
            enote::document::DocumentFormat::EnoteMarkdown
        );
        edit(&mut app, 'a');
        assert!(app.buffers.entries[&id].modified());
        let saved = app.buffers.save_job(id, None).unwrap().write().unwrap();
        let _ = app.update(Message::Saved(Ok(saved)));
        assert!(!app.buffers.entries[&id].modified());
        assert_eq!(
            std::fs::read_to_string(root.join("Research/Note.mde")).unwrap(),
            "a"
        );
        let _ = app.update(Message::Opened(Some(LoadedFile::read(
            &root.join("Research/Note.mde"),
        ))));
        assert_eq!(app.active, Some(id));
        assert_eq!(app.buffers.entries.len(), 1);
    }

    #[test]
    fn cancel_creation_writes_nothing_and_hover_exit_does_not_clear_another_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Enote::default();
        let opened = OpenedVolume::create(dir.path(), "Notes", &app.base_settings).unwrap();
        let root = opened.storage.root.clone();
        app.apply_volume(opened).unwrap();
        let _ = app.update(Message::New);
        let _ = app.update(Message::CancelEntry);
        assert!(app.entry_draft.is_none());
        assert!(!root.join("Untitled.mde").exists());
        let first = root.join("first");
        let second = root.join("second");
        let _ = app.update(Message::HoverFolder(first.clone(), true));
        let _ = app.update(Message::HoverFolder(second.clone(), true));
        let _ = app.update(Message::HoverFolder(first, false));
        assert_eq!(app.hovered_folder, Some(second));
        let _ = app.update(Message::CursorMoved(iced::Point::new(10.0, 200.0)));
        assert!(app.hovered_folder.is_none());
    }

    #[test]
    fn configured_shortcut_undo_and_tab_navigation_work() {
        let mut app = Enote::default();
        app.new_buffer();
        let first = app.active.unwrap();
        edit(&mut app, 'a');
        app.settings
            .shortcuts
            .0
            .insert(ShortcutAction::Undo, vec!["Alt+U".into()]);
        let _ = app.update(Message::KeyPressed("u".into(), keyboard::Modifiers::ALT));
        assert_eq!(app.buffers.entries[&first].text(), Some(""));
        app.new_buffer();
        let second = app.active.unwrap();
        let _ = app.update(Message::NextTab);
        assert_eq!(app.active, Some(first));
        let _ = app.update(Message::PreviousTab);
        assert_eq!(app.active, Some(second));
    }
}
