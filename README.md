# eNote

eNote — локальный менеджер заметок на Rust с интерфейсом на iced,
вдохновлённый Obsidian и минимализмом Typora. Заметки, книги и медиа
хранятся обычными файлами внутри вольюма — выбранной пользователем папки.
Приложение не требует облачного сервиса или собственной базы для документов.

Сейчас реализованы дерево файлов, создание и перемещение объектов, вкладки,
редактирование текста с сохранением и undo/redo, разделение рабочего пространства,
просмотр изображений и документов. Вкладки и панели поддерживают drag-and-drop,
а рабочее пространство восстанавливается отдельно для каждого вольюма.
PDF отображается через Hayro, CBZ — через Rust-кодеки изображений,
EPUB и FB2 — через текстовый рендерер на cosmic-text. Вьювер поддерживает
непрерывную прокрутку, масштаб и запоминание позиции; его нижняя панель
появляется при наведении.
EPUB/FB2 пока отображают текст без CSS и встроенных иллюстраций.
MOBI и XPS/OXPS больше не поддерживаются.

Новые заметки используют расширение `.mde` (eNote Markdown); сейчас это
совместимый с Markdown UTF-8 текст. WYSIWYG и расширенные возможности формата
пока в разработке. Мобильные платформы входят в направление развития,
но готовых Android/iOS-сборок и мобильных файловых диалогов пока нет.

## Работа с вкладками и панелями

Откройте вольюм через значок папки, затем выберите файл в проводнике.
Файл открывается во вкладке; повторное открытие выбирает существующий буфер.
Редактор не содержит нижней информационной панели.

- Перетаскивайте вкладки в верхней полоске, чтобы менять их порядок.
- Сбросьте вкладку в центр панели, чтобы перенести туда буфер.
- Сбросьте вкладку к левому, правому, верхнему или нижнему краю панели,
  чтобы разместить буфер в разделённом рабочем пространстве.
- Перетаскивайте панель за её заголовок: сброс в центр другой панели меняет
  панели местами, сброс к краю меняет их расположение.
- Перетаскивайте разделители для изменения размеров панелей.
- Escape или отпускание вкладки вне цели отменяет перенос.

Перенос сохраняет текст, историю редактирования и состояние открытого буфера.
Порядок вкладок, активный буфер, расположение панелей и пропорции разделений
запоминаются для каждого вольюма. Размер и положение окна запоминаются
для приложения и сохраняются при переключении между вольюмами.
Состояние записывается в фоне, а также перед выходом или сменой вольюма.
Восстановление открывает сохранённые версии файлов; несохранённые правки
и безымянные буферы после перезапуска не восстанавливаются.

## Просмотр документов

Поддерживаются PDF, CBZ, EPUB и FB2. Страницы рендерятся в отдельном потоке:
PDF разбирается один раз, а EPUB/FB2 повторно используют подготовленный текст,
шрифты и пагинацию. Ближайшие страницы загружаются заранее; кэш ограничен
по памяти. Устаревшие запросы пропускаются при быстрой прокрутке и изменении
масштаба. Уже загруженные страницы остаются видимыми до готовности замены.
Скрытые вкладки документов начинают рендериться при показе в панели.

PDF-движок Hayro ещё развивается: сложные PDF могут отображаться иначе,
чем в MuPDF. Ввод пароля для защищённых документов пока не реализован.
EPUB/FB2 отображают текст без CSS, встроенных иллюстраций и книжных шрифтов;
поддерживаются XML-кодировки, включая Windows-1251, и UTF-8/UTF-16 BOM.
Страницы CBZ упорядочиваются по числам в именах: `page2` идёт перед `page10`.
MOBI и XPS/OXPS не поддерживаются.

## Сборка

Команды выполняются из корня проекта. Установите актуальный стабильный Rust
и Cargo через [rustup](https://rust-lang.org/tools/install/).
Проект использует Rust edition 2024. Зависимости закреплены в `Cargo.lock`;
используйте `--locked`, чтобы сборка не меняла их версии.

Рендерер PDF Hayro, разбор EPUB/FB2 и кодеки CBZ написаны на Rust.
MuPDF, bindgen, libclang и сборка C/C++-библиотек больше не нужны.
Минимальная версия Rust — 1.92. Первая сборка требует доступа к crates.io.
Оконный интерфейс использует системные API, графический драйвер и шрифты ОС.

### macOS

Установите Xcode Command Line Tools:

```sh
xcode-select --install
```

После завершения установки и установки Rust:

```sh
cargo build --release --locked
./target/release/enote
```

Сборка по умолчанию предназначена для архитектуры текущего компьютера:
Apple Silicon или Intel. Xcode Command Line Tools нужны для системного
SDK и линкера, а не для сборки PDF-движка.

Cargo создаёт исполняемый файл, а не подписанный `.app` или `.dmg`.
Локальные тестовые `.app` внутри `target/` не входят в репозиторий.

### Windows (MSVC)

Установите Rust через rustup и рекомендованные им MSVC Build Tools / Windows SDK
для системного линкера. LLVM и libclang не требуются.

```powershell
cargo build --release --locked
.\target\release\enote.exe
```

Cargo создаёт исполняемый файл; готовый установщик не создаётся.
Подготовка линкера описана в
[документации Rust](https://rust-lang.github.io/rustup/installation/windows-msvc.html).

### Linux (Ubuntu / Debian)

Установите инструменты сборки и библиотеки оконной системы и шрифтов:

```sh
sudo apt update
sudo apt install build-essential libx11-6 libxcursor1 libxi6 libxrandr2 \
  libxkbcommon0 libegl1 fonts-dejavu-core xwayland
```

После установки Rust:

```sh
cargo build --release --locked
./target/release/enote
```

Для запуска нужна графическая сессия X11 или Wayland с XWayland и рабочий
графический драйвер. Нативный Wayland отключён, чтобы исключить C-shim из
зависимостей настольной сборки. Файловые диалоги используют XDG Desktop Portal:
установите
`xdg-desktop-portal` и подходящий backend рабочего окружения
(например, `xdg-desktop-portal-gtk` для GTK-окружения).
В других дистрибутивах установите эквивалентные пакеты.

### Разработка и проверки

```sh
cargo run --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

Локально проверены сборка и GUI на macOS. Инструкции для Windows и Linux
основаны на зависимостях проекта; сборки на этих ОС пока не проверены.
Собирать следует нативно на нужной ОС: установка Rust target сама по себе
не заменяет системный SDK и линкер целевой ОС.

## Хранение и настройки

В корне вольюма находится скрытая папка `.enote/`: идентификатор вольюма,
его настройки, рабочее пространство, позиции чтения и корзина. Проводник приложения её скрывает.
Последний открытый вольюм восстанавливается при запуске вместе с вкладками,
активным буфером и расположением панелей. Положение и размер окна также
запоминаются. Панели буферов можно перетаскивать за заголовок: в центр другой
панели для обмена местами или к краю для изменения расположения.

Настройки объединяются в порядке: встроенные → глобальные → пользовательские
→ настройки текущего вольюма. Пользовательский файл на macOS/Linux:
`$HOME/.config/enote/settings.json` (или `$XDG_CONFIG_HOME/enote/settings.json`);
на Windows: `%APPDATA%\enote\settings.json`.
Настройки вольюма: `.enote/settings.json`.

Например, отключить запоминание позиции чтения:

```json
{
  "viewer": {
    "remember_position": false
  }
}
```

Все доступные параметры показаны в [settings.example.json](settings.example.json),
шорткаты — в [shortcuts.default.json](shortcuts.default.json).
Изменённые JSON-настройки применяются при следующем запуске;
настройки вольюма также перечитываются при его повторном открытии.

Исходники находятся в `src/`, тестовый PDF — в `tests/fixtures/`.
Рабочая ветка репозитория — `develop`. Сборочные артефакты, техническое
состояние локальных вольюмов и файлы ОС исключены через `.gitignore`.
Личные документы не следует хранить внутри репозитория проекта.

Hayro распространяется под MIT OR Apache-2.0.
Сведения о зависимости: [THIRD_PARTY.md](THIRD_PARTY.md).

Ниже приведена подробная техническая справка по текущей реализации.

## Settings

Settings are JSON objects, loaded in this order: built-in defaults, global
settings, user settings, volume settings. Nested objects merge recursively;
volume values win for the current session.
An absent file inherits defaults. An invalid layer is skipped with a warning;
other layers continue loading. Unknown keys are retained by the storage API.

Application shortcuts live in the `shortcuts` settings object. Defaults are
defined in `shortcuts.default.json`, embedded into the application, and shown in
`settings.example.json`. Override any command in a global, user or volume file:

```json
{
  "shortcuts": {
    "save": ["Alt+S"],
    "redo": []
  }
}
```

`Command` means Cmd on macOS and Ctrl elsewhere. Explicit Ctrl, Cmd/Meta, Alt
and Shift modifiers are supported; `[]` disables a command. Bindings require
an exact modifier match. Invalid or duplicate combinations reject the settings
layer with a warning. Volume overrides reload when reopening the volume;
global and user changes require restarting the application.

Linux and macOS paths:

- Global: `/etc/enote/settings.json`.
- User: `$XDG_CONFIG_HOME/enote/settings.json`, when XDG_CONFIG_HOME is absolute;
  otherwise `$HOME/.config/enote/settings.json`.

Windows paths:

- Global: `%PROGRAMDATA%\enote\settings.json`.
- User: `%APPDATA%\enote\settings.json`, with `%USERPROFILE%\.config\enote\settings.json`
  as a fallback when APPDATA is unavailable.

The first launch creates the user directory and a `settings.json` containing
`{}`, which inherits global and built-in defaults. Global files are read-only
from the application's perspective and are never created automatically.
See `settings.example.json` for the available fields. Window dimensions,
editor font size, sidebar visibility and width are applied at startup.

`SettingsStore::update_user` merges a JSON patch into the user file and saves
it atomically. It preserves unrelated keys and refuses to overwrite malformed
settings. Reader position preferences are configured in JSON settings.
Configuration reload, a settings UI, and mobile configuration-directory
integration remain future work.

## Buffers

The `enote` library (`lib.rs`) exposes `buffer` and `volume` without UI types.
`buffer.rs` owns document state independently of the UI. Each buffer has a stable
ID, optional file reference, typed content, read-only flag, change counter,
saved snapshot, and bounded undo/redo history. A file opens in one buffer;
opening it again selects that buffer without replacing unsaved edits.

`volume.rs` maps a volume ID and relative path to a local file. Resolution rejects
parent traversal and symlinks outside the root. The application restricts document
opening and saving to the active volume. The right-hand explorer displays its tree.

`app.rs` displays buffers in resizable split panes and retains editor state for hidden buffers.
`file_dialog.rs` isolates desktop dialogs; mobile pickers remain unimplemented.
A new volume starts with an empty workspace and a compact file explorer on the right.
Reopening a volume restores its last tabs, active buffer and split-pane layout.
Open a volume with the folder icon; directories expand lazily and files open
in buffers. Hover a folder (including the root) to show New file and New folder
icons. Enter a name inline and confirm; cancel creates nothing. On narrow/touch
layouts the icons remain visible. The outer Files icon toggles the
panel. On narrow windows the panel occupies the workspace until a file is opened.
Tabs appear only for open buffers, and closing the last tab restores the empty
workspace. Editing is frameless, without a toolbar or permanent status bar.
New file/folder, open, save, save as, close, undo/redo and tab navigation have
configurable shortcuts. The tab header provides Save and a File actions menu.
Unsaved changes require
confirmation before closing a buffer or the application.

Text files must be UTF-8. Images have a read-only preview. Audio, video, and
binary files have read-only metadata views; playback is not implemented.
Text and encoded images are limited to 32 MiB, decoded images to 64 MiB.
Undo and redo each retain up to 100 snapshots or 64 MiB.

PDF, EPUB, FB2 and CBZ open as read-only document buffers. PDF pages use
[Hayro](https://github.com/LaurenzV/hayro), a pure Rust renderer; CBZ uses `zip`
and `image`; EPUB/FB2 text is parsed with `roxmltree` and shaped/rasterized with
`cosmic-text` and Swash. EPUB chapters follow the package spine. Book pagination
uses fixed logical pages, independent of zoom. The reader retains continuous
scrolling, zoom and saved positions.

EPUB/FB2 currently display text with paragraph boundaries and Unicode shaping;
CSS styling, embedded illustrations and book fonts are not rendered. XML
encoding declarations (including Windows-1251) and UTF-8/UTF-16 BOMs are supported.
MOBI and XPS/OXPS are no longer supported. Hayro is still evolving, so complex PDFs
can render differently from MuPDF; password entry is not implemented.

Desktop dependency trees for macOS, Windows and Linux contain no C/C++ build
libraries (`cc`, `cmake`, `bindgen`, `mupdf-sys`, `freetype-sys`). Linux uses
X11/XWayland. System windowing APIs, graphics drivers and linkers remain required.
`Cargo.lock` also records inactive Android dependencies of winit; Android is not
part of this desktop-only guarantee.

Hayro is licensed under MIT OR Apache-2.0; see `THIRD_PARTY.md`.

The reader stores page, zoom and scroll position in
`.enote/viewer-state.json`. Positions follow files when moved or renamed inside
eNote. Disable remembering with `{"viewer":{"remember_position":false}}` in
global, user or volume settings. Reopening then starts
at the first page; existing saved positions are retained.

Use the split-right or split-below icons in a pane header to create another
area. Open a file from the explorer or choose a tab for the focused pane.
Selecting an already visible buffer focuses its existing pane. Drag pane
dividers to resize them; closing a pane keeps its buffer and unsaved edits.
Drag the left edge of the file explorer to change its width for the session.

The document reader scrolls continuously through pages and prefetches a bounded
window ahead of the viewport. Rendering starts when a document is shown in a pane;
a dedicated worker then reuses PDF parsing/interpreter caches or EPUB/FB2
layout/font caches between pages. Obsolete queued requests are skipped during fast scrolling, resizing or zooming.
Rendered image handles are reused; pixel buffers are shared without UI-thread copies.
CPU bitmap caches are bounded to 64 MiB in the worker and 128 MiB in the viewer
(shared buffers count toward both budgets); at most 16 pages are retained per cache.
Old-resolution pages stay visible until replacement images arrive. Output sizes are
bucketed in 64-pixel steps to avoid rerendering for every small resize.
Development builds optimize dependencies so CPU rendering also stays fast with
`cargo run`; the first build after changing this profile recompiles dependencies.

The compact, single-row controls appear at the bottom only while the pointer is
over the viewer; very narrow panes allow horizontal scrolling of those controls.

Run the optional repeat-navigation microbenchmark with:

```sh
cargo test --locked --lib benchmark_repeated_document_pages -- --ignored --nocapture
```

Files created without an extension use `.mde` (eNote Markdown). `.md` remains
ordinary Markdown. `.mde` currently stores compatible UTF-8 Markdown text and has
a distinct document format for future eNote extensions; extended syntax and
WYSIWYG rendering are not implemented yet. New files open in a focused editor
tab. Existing names are never overwritten during creation. Save as preserves
the original file and changes the active buffer's source to the chosen copy.

File I/O runs through iced tasks. Saving writes a temporary file in the same
directory and atomically replaces the destination. It checks the saved snapshot
against disk before writing, but does not lock files against concurrent writers.
Edits made while saving remain marked as modified.

## File Manager

The explorer has collapsible folders, nesting guides, folders-first sorting,
and horizontal scrolling for deep trees. Right-click an object to open its
context menu. Files support Open, Rename, Move to, Duplicate, Copy path and
Move to trash. Folders also support New file, New folder, Refresh and Collapse
all. The volume root cannot be renamed, moved, duplicated or deleted.

Move files or entire folders by dropping them onto another folder, including
the root, or use Move to and choose a destination inside the volume. Renaming
is inline; a file's extension is preserved when the new name has none.
Drag-and-drop starts after a five-pixel movement, shows the dragged object's
name beside the cursor and outlines valid destination folders. Release on a
folder row to move, or on the empty explorer area to move to the volume root.
The root title also accepts drops. Escape or release outside the explorer
cancels the gesture. Folder and file rows take priority over the root drop area.
Dropping onto a file, the source, its descendants or its current parent is a
no-op. Ordinary clicks still open files or expand folders.
Folder creation buttons appear inside the hovered folder row and do not reserve
a separate column. Files dropped from the system file manager are copied into
the hovered folder (or the volume root), with originals preserved. Multiple
files are imported sequentially; existing names are never overwritten.
Importing external folders is not supported yet.
Moving and renaming update paths in open buffers without losing unsaved text,
cursor positions, undo/redo history or expanded subtrees. Name collisions,
moving a folder into itself and operations outside the volume are rejected.

Deletion requires confirmation and moves the object into `.enote/trash`, not
the OS trash. Open buffers for deleted objects are closed; the confirmation
warns when this discards unsaved edits. The root context menu has Restore last
deleted, also available after restarting. Recovery restores the saved-on-disk
content to its original path and never overwrites an existing object. Its
parent folder must exist. Trash records use relative path components and the
volume UUID, without machine-specific absolute paths. External filesystem
changes are refreshed manually; a filesystem watcher is not yet implemented.

Markdown WYSIWYG, mobile file pickers, and recovery of unsaved edits
are future work. The current GUI and native dialogs are verified on macOS.

## Volumes

A volume is an ordinary directory tree containing notes and media. Create a
new volume by entering its name and choosing a parent directory, or open an
existing directory. Opening an existing directory initializes `.enote` without
changing existing documents.

The root contains a hidden `.enote` directory:

- `volume.json`: format version and persistent UUID, preserved when moved.
- `settings.json`: JSON overrides for this volume; `{}` inherits user settings.
- `workspace-state.json`: open file tabs, their order, active buffer and pane layout.
- `viewer-state.json`: document pages, zoom and scroll positions when remembering is enabled.
- `trash/`: deleted files and their restoration records.

These files contain portable JSON without absolute machine paths. The file
explorer hides `.enote`, and technical files cannot be opened or saved as notes.
The application refuses unsupported manifest versions or linked technical files.
Volume settings never modify user settings; switching volumes starts again from
the global/user base. Font size and panel settings apply immediately. Window
geometry is restored from application state at startup and remains unchanged
when switching volumes.

The user configuration directory contains a separate `state.json` with the last
successfully opened volume's path and the window's logical size/position. eNote
restores both at startup. With no remembered volume, or an unavailable one, it offers Create volume and Open
volume. Failed restoration reports an error without deleting the remembered path.
Switching volumes requires confirmation for unsaved changes; cancellation or
failure preserves the current buffers.

Each volume saves open file tabs, the active buffer, pane arrangement and split
ratios in `.enote/workspace-state.json`, using only relative document paths.
Changes are saved in the background after a short delay, with a final flush on
exit or volume switch. Missing or unavailable files are skipped during restoration;
invalid state files are preserved. Session restoration reopens saved file contents;
unsaved edits and untitled scratch buffers are not recovered.

Drag tabs in the top strip to reorder them, into a pane to move the buffer,
or to a pane edge to create a split. The tab order is included in the saved
workspace. Drop hints indicate the selected target; Escape cancels the drag.

Drag a pane by its title bar: drop in another pane's center to swap buffers, on a
pane edge to rearrange the split, or on the workspace edge to move it there.
Controls in the title bar remain clickable and are excluded from the drag region.

Run the model and UI-state tests with `cargo test`.
