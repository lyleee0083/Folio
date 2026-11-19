#![cfg_attr(not(test), windows_subsystem = "windows")]

mod document;
mod files;
mod icons;
mod math;
mod performance;
mod platform;
mod preview;
mod raw;

use crate::files::{wide, State, Store};
use crate::platform::{Election, WM_LAUNCH};
use crate::preview::{Hit, Page, Renderer};
use percent_encoding::percent_decode_str;
use std::cell::RefCell;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::SystemServices::{SS_ENDELLIPSIS, SS_PATHELLIPSIS, SS_RIGHT};
use windows::Win32::UI::Controls::RichEdit::{
    CFM_COLOR, CHARFORMATW, CHARRANGE, EM_EXGETSEL, EM_EXSETSEL, EM_SETBKGNDCOLOR,
    EM_SETCHARFORMAT, EM_SETEVENTMASK, ENM_CHANGE, SCF_ALL,
};
use windows::Win32::UI::Controls::{
    SetScrollInfo, DRAWITEMSTRUCT, EM_GETMODIFY, EM_GETRECT, EM_SCROLLCARET, EM_SETCUEBANNER,
    EM_SETLIMITTEXT, EM_SETMODIFY, EM_SETRECTNP, EM_SETSEL,
};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{
    DefSubclassProc, DragAcceptFiles, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW, HDROP,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const WINDOW: PCWSTR = w!("FolioNativeWindow");
const PREVIEW: PCWSTR = w!("FolioDocumentPreview");
const GUTTER: PCWSTR = w!("FolioRawLineNumbers");
const FILTER_HINT: &str = "筛选文档";
const TIMER: usize = 1;
const RAW_COLOR_TIMER: usize = 2;
const OPEN: u16 = 10;
const FOLDER: u16 = 11;
const RAW: u16 = 12;
const SAVE: u16 = 13;
const SEARCH: u16 = 14;
const OUTLINE: u16 = 15;
const THEME: u16 = 16;
const SETTINGS: u16 = 17;
const MORE: u16 = 18;
const LIBRARY: u16 = 19;
const FILTER: u16 = 20;
const DOCUMENTS: u16 = 21;
const HEADINGS: u16 = 22;
const QUERY: u16 = 23;
const PREVIOUS: u16 = 24;
const NEXT: u16 = 25;
const CLOSE_SEARCH: u16 = 26;
const EDITOR: u16 = 27;
const REFRESH: u16 = 30;
const FOCUS: u16 = 31;
const COPY: u16 = 32;
const PRINT: u16 = 33;
const FONT_INCREASE: u16 = 35;
const FONT_DECREASE: u16 = 36;
const WIDTH_AUTO: u16 = 37;
const WIDTH_NARROW: u16 = 38;
const WIDTH_WIDER: u16 = 39;
const SPACING_INCREASE: u16 = 40;
const SPACING_DECREASE: u16 = 41;
const FONT_SANS: u16 = 42;
const FONT_SERIF: u16 = 43;
const FONT_MONO: u16 = 44;

#[derive(Default)]
struct Controls {
    toolbar: Vec<(u16, HWND, i32)>,
    preview: HWND,
    title: HWND,
    meta: HWND,
    docs: HWND,
    filter: HWND,
    heads: HWND,
    heads_title: HWND,
    open: HWND,
    folder: HWND,
    editor: HWND,
    gutter: HWND,
    status: HWND,
    metrics: HWND,
    query: HWND,
    prev: HWND,
    next: HWND,
    clear: HWND,
    matches: HWND,
}

struct App {
    window: HWND,
    controls: Controls,
    dpi: u32,
    font: HFONT,
    filter_font: HFONT,
    edit_font: HFONT,
    gutter_font: HFONT,
    surface: HBRUSH,
    rail_surface: HBRUSH,
    shell: icons::Shell,
    raw_lines: raw::LineIndex,
    store: Store,
    state: State,
    active: Option<usize>,
    docs_visible: Vec<usize>,
    raw: bool,
    focus: bool,
    search: bool,
    dragging: bool,
    horizontal_drag: Option<usize>,
    pending_link: Option<String>,
    raw_query: String,
    editor_base: String,
    preview_size: (i32, i32),
    document: document::Document,
    page: Page,
    renderer: Renderer,
    icons: icons::Painter,
    queue: platform::LaunchQueue,
    measurement: Option<performance::Measurement>,
}

impl App {
    fn new(store: Store) -> Result<Self> {
        let mut state = store.load();
        state.documents.retain_mut(|entry| {
            if entry.source_path.as_os_str().is_empty() || !files::is_markdown(&entry.source_path) {
                return false;
            }
            match files::open(&entry.source_path) {
                Ok(mut current) => {
                    current.id = entry.id.clone();
                    current.reading_progress = entry.reading_progress;
                    *entry = current;
                    true
                }
                Err(error) => {
                    error.kind() != std::io::ErrorKind::NotFound
                        && entry.content.len() <= files::FILE_LIMIT
                }
            }
        });
        let active = state
            .active_id
            .as_ref()
            .and_then(|id| state.documents.iter().position(|item| &item.id == id))
            .or_else(|| state.documents.len().checked_sub(1));
        Ok(Self {
            window: HWND::default(),
            controls: Controls::default(),
            dpi: 96,
            font: HFONT::default(),
            filter_font: HFONT::default(),
            edit_font: HFONT::default(),
            gutter_font: HFONT::default(),
            surface: HBRUSH::default(),
            rail_surface: HBRUSH::default(),
            shell: icons::Shell::default(),
            raw_lines: raw::LineIndex::new(""),
            store,
            state,
            active,
            docs_visible: vec![],
            raw: false,
            focus: false,
            search: false,
            dragging: false,
            horizontal_drag: None,
            pending_link: None,
            raw_query: String::new(),
            editor_base: String::new(),
            preview_size: (1, 1),
            document: document::Document::default(),
            page: Page::default(),
            renderer: Renderer::new()?,
            icons: icons::Painter::new()?,
            queue: Default::default(),
            measurement: None,
        })
    }

    fn scale(&self, value: i32) -> i32 {
        ((value as f32) * self.dpi as f32 / 96.).round() as i32
    }
    fn dip(&self, pixels: i32) -> f32 {
        pixels as f32 * 96. / self.dpi as f32
    }

    unsafe fn create_controls(&mut self, pointer: isize) -> Result<()> {
        self.dpi = GetDpiForWindow(self.window).max(96);
        LoadLibraryW(w!("Msftedit.dll"))?;
        let c = &mut self.controls;
        let parent = self.window;
        for (id, label, width) in [
            (LIBRARY, "文档库", 36),
            (RAW, "编辑", 36),
            (SAVE, "保存", 36),
            (SEARCH, "查找", 36),
            (OUTLINE, "大纲", 36),
            (THEME, "主题", 36),
            (SETTINGS, "设置", 36),
            (MORE, "更多", 36),
        ] {
            c.toolbar.push((
                id,
                child(
                    parent,
                    w!("BUTTON"),
                    label,
                    WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
                    id,
                )?,
                width,
            ));
        }
        c.title = child(
            parent,
            w!("STATIC"),
            "Folio",
            WINDOW_STYLE(SS_ENDELLIPSIS.0),
            0,
        )?;
        c.meta = child(parent, w!("STATIC"), "", WINDOW_STYLE(SS_PATHELLIPSIS.0), 0)?;
        c.open = child(
            parent,
            w!("BUTTON"),
            "打开",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            OPEN,
        )?;
        c.folder = child(
            parent,
            w!("BUTTON"),
            "导入",
            WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32),
            FOLDER,
        )?;
        c.filter = child(
            parent,
            w!("EDIT"),
            "",
            WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_TABSTOP,
            FILTER,
        )?;
        send(
            c.filter,
            EM_SETCUEBANNER,
            0,
            wide(FILTER_HINT).as_ptr() as isize,
        );
        let _ = SetWindowSubclass(c.filter, Some(filter_proc), 0, 0);
        c.docs = child(
            parent,
            w!("LISTBOX"),
            "",
            WINDOW_STYLE(
                LBS_NOTIFY as u32
                    | LBS_NOINTEGRALHEIGHT as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32,
            ) | WS_VSCROLL
                | WS_TABSTOP,
            DOCUMENTS,
        )?;
        c.heads_title = child(parent, w!("STATIC"), "大纲目录", WINDOW_STYLE(0), 0)?;
        c.heads = child(
            parent,
            w!("LISTBOX"),
            "",
            WINDOW_STYLE(
                LBS_NOTIFY as u32
                    | LBS_NOINTEGRALHEIGHT as u32
                    | LBS_OWNERDRAWFIXED as u32
                    | LBS_HASSTRINGS as u32,
            ) | WS_VSCROLL
                | WS_TABSTOP,
            HEADINGS,
        )?;
        c.preview = child(parent, PREVIEW, "Markdown 预览", WS_VSCROLL | WS_TABSTOP, 0)?;
        SetWindowLongPtrW(c.preview, GWLP_USERDATA, pointer);
        c.editor = child(
            parent,
            w!("RICHEDIT50W"),
            "",
            WINDOW_STYLE(ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_NOHIDESEL as u32)
                | WS_VSCROLL
                | WS_TABSTOP,
            EDITOR,
        )?;
        send(c.editor, EM_SETLIMITTEXT, 11 * 1024 * 1024, 0);
        send(c.editor, EM_SETEVENTMASK, 0, ENM_CHANGE as isize);
        c.gutter = child(parent, GUTTER, "行号", WINDOW_STYLE(0), 0)?;
        SetWindowLongPtrW(c.gutter, GWLP_USERDATA, pointer);
        SetWindowSubclass(c.editor, Some(raw::editor_proc), 0, c.gutter.0 as usize).ok()?;
        c.query = child(
            parent,
            w!("EDIT"),
            "",
            WINDOW_STYLE(ES_AUTOHSCROLL as u32) | WS_BORDER | WS_TABSTOP,
            QUERY,
        )?;
        c.prev = child(parent, w!("BUTTON"), "‹", WS_TABSTOP, PREVIOUS)?;
        c.next = child(parent, w!("BUTTON"), "›", WS_TABSTOP, NEXT)?;
        c.clear = child(parent, w!("BUTTON"), "×", WS_TABSTOP, CLOSE_SEARCH)?;
        c.matches = child(parent, w!("STATIC"), "", WINDOW_STYLE(0), 0)?;
        c.status = child(
            parent,
            w!("STATIC"),
            "就绪",
            WINDOW_STYLE(SS_ENDELLIPSIS.0),
            0,
        )?;
        c.metrics = child(parent, w!("STATIC"), "", WINDOW_STYLE(SS_RIGHT.0), 0)?;
        DragAcceptFiles(c.preview, true);
        DragAcceptFiles(c.editor, true);
        DragAcceptFiles(parent, true);
        self.update_fonts();
        self.update_theme();
        self.refresh_list(false);
        self.load_active();
        Ok(())
    }

    unsafe fn update_fonts(&mut self) {
        let create_font =
            |height, family| CreateFontW(height, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 0, 0, family);
        let ui = create_font(-self.scale(13), w!("Segoe UI"));
        let filter = create_font(-self.scale(12), w!("Microsoft YaHei UI"));
        let editor = create_font(-self.scale(15), w!("Consolas"));
        let numbers = create_font(-self.scale(12), w!("Consolas"));
        let previous = [
            std::mem::replace(&mut self.font, ui),
            std::mem::replace(&mut self.filter_font, filter),
            std::mem::replace(&mut self.edit_font, editor),
            std::mem::replace(&mut self.gutter_font, numbers),
        ];
        let c = &self.controls;
        for list in [c.docs, c.heads] {
            send(list, LB_SETITEMHEIGHT, 0, self.scale(28) as isize);
        }
        for hwnd in c.toolbar.iter().map(|entry| entry.1).chain([
            c.title,
            c.meta,
            c.docs,
            c.heads,
            c.heads_title,
            c.open,
            c.folder,
            c.query,
            c.prev,
            c.next,
            c.clear,
            c.matches,
            c.status,
            c.metrics,
        ]) {
            send(hwnd, WM_SETFONT, self.font.0 as usize, 1);
        }
        send(c.filter, WM_SETFONT, self.filter_font.0 as usize, 1);
        send(c.editor, WM_SETFONT, self.edit_font.0 as usize, 1);
        for font in previous {
            if !font.is_invalid() {
                let _ = DeleteObject(font);
            }
        }
    }

    unsafe fn update_theme(&mut self) {
        if self.state.options.is_dark {
            let _ = SetPropW(self.window, w!("FolioDarkTheme"), HANDLE(1 as *mut c_void));
        } else {
            let _ = RemovePropW(self.window, w!("FolioDarkTheme"));
        }
        if !self.surface.is_invalid() {
            let _ = DeleteObject(self.surface);
        }
        if !self.rail_surface.is_invalid() {
            let _ = DeleteObject(self.rail_surface);
        }
        let tone = icons::Tone::new(self.state.options.is_dark);
        self.surface = CreateSolidBrush(COLORREF(if self.state.options.is_dark {
            0x24211e
        } else {
            0xffffff
        }));
        self.rail_surface = CreateSolidBrush(win_color(tone.rail));
        send(
            self.controls.editor,
            EM_SETBKGNDCOLOR,
            0,
            if self.state.options.is_dark {
                0x24211e
            } else {
                0xffffff
            },
        );
        self.raw_colors();
        repaint_window(self.window);
    }

    unsafe fn raw_colors(&self) {
        let color = if self.state.options.is_dark {
            0xf2eeec
        } else {
            0x24201e
        };
        let format = CHARFORMATW {
            cbSize: size_of::<CHARFORMATW>() as u32,
            dwMask: CFM_COLOR,
            crTextColor: COLORREF(color),
            ..Default::default()
        };
        send(
            self.controls.editor,
            EM_SETCHARFORMAT,
            SCF_ALL as usize,
            &format as *const _ as isize,
        );
        if !self.raw {
            send(self.controls.editor, EM_SETMODIFY, 0, 0);
        }
    }

    unsafe fn arrange(&mut self) {
        let r = client(self.window);
        let width = r.right;
        let height = r.bottom;
        let top = self.scale(44);
        let foot = self.scale(24);
        let sidebar = if self.state.sidebar_open && !self.focus {
            self.scale(218).min(width / 3)
        } else {
            0
        };
        let outline = if self.state.outline_open && !self.focus && !self.raw {
            self.scale(205).min(width / 4)
        } else {
            0
        };
        let side_gap = self.scale(8);
        self.shell = icons::Shell {
            sidebar,
            outline,
            toolbar_bottom: top,
            footer_top: (height - foot).max(top),
            filter: (sidebar > 0).then_some(RECT {
                left: side_gap,
                top: top + self.scale(43),
                right: sidebar - side_gap,
                bottom: top + self.scale(69),
            }),
        };
        let c = &self.controls;
        let bar_height = self.scale(28);
        let mut right = width - side_gap;
        for (id, button, size) in c.toolbar.iter().rev() {
            if *id == LIBRARY || (*id == SAVE && !self.raw) {
                if *id != LIBRARY {
                    show(*button, false);
                }
                continue;
            }
            let bw = self.scale(*size);
            right -= bw;
            place(
                *button,
                right,
                self.scale(7),
                bw - self.scale(4),
                bar_height,
                true,
            );
        }
        if let Some((_, sidebar_button, _)) = c.toolbar.iter().find(|(id, _, _)| *id == LIBRARY) {
            place(
                *sidebar_button,
                side_gap,
                self.scale(7),
                self.scale(32),
                bar_height,
                true,
            );
        }
        let name_x = self.scale(48);
        let meta = (right - name_x).max(0) / 3;
        place(
            c.title,
            name_x,
            self.scale(13),
            (right - name_x - meta).max(0),
            bar_height,
            true,
        );
        place(
            c.meta,
            right - meta,
            self.scale(13),
            (meta - side_gap).max(0),
            bar_height,
            meta > self.scale(85),
        );
        place(
            c.open,
            side_gap,
            top + side_gap,
            (sidebar / 2 - side_gap).max(0),
            bar_height,
            sidebar > 0,
        );
        place(
            c.folder,
            sidebar / 2 + side_gap / 2,
            top + side_gap,
            (sidebar / 2 - side_gap * 2).max(0),
            bar_height,
            sidebar > 0,
        );
        place(
            c.filter,
            side_gap + self.scale(8),
            top + self.scale(48),
            (sidebar - side_gap * 2 - self.scale(16)).max(0),
            self.scale(18),
            sidebar > 0,
        );
        place(
            c.docs,
            side_gap,
            top + self.scale(76),
            (sidebar - side_gap * 2).max(0),
            (height - top - foot - self.scale(80)).max(0),
            sidebar > 0,
        );
        place(
            c.heads_title,
            width - outline + side_gap,
            top + side_gap,
            outline - side_gap,
            self.scale(22),
            outline > 0,
        );
        place(
            c.heads,
            width - outline + side_gap,
            top + self.scale(33),
            outline - side_gap * 2,
            (height - top - foot - self.scale(38)).max(0),
            outline > 0,
        );
        let search_y = if self.search { self.scale(37) } else { 0 };
        let main_width = (width - sidebar - outline).max(1);
        let main_height = (height - top - search_y - foot).max(1);
        place(
            c.preview,
            sidebar,
            top + search_y,
            main_width,
            main_height,
            !self.raw,
        );
        let gutter = self.scale(54);
        place(
            c.gutter,
            sidebar,
            top + search_y,
            gutter,
            main_height,
            self.raw,
        );
        place(
            c.editor,
            sidebar + gutter,
            top + search_y,
            (main_width - gutter).max(1),
            main_height,
            self.raw,
        );
        if self.raw {
            let bounds = client(c.editor);
            let inset = self.scale(8);
            let text_area = RECT {
                left: inset,
                top: inset,
                right: (bounds.right - self.scale(12)).max(inset + 1),
                bottom: (bounds.bottom - inset).max(inset + 1),
            };
            send(c.editor, EM_SETRECTNP, 0, &text_area as *const _ as isize);
        }
        let search_left = (sidebar + main_width - self.scale(384)).max(sidebar + side_gap);
        place(
            c.query,
            search_left,
            top + self.scale(6),
            self.scale(210),
            self.scale(25),
            self.search,
        );
        place(
            c.matches,
            search_left + self.scale(217),
            top + self.scale(9),
            self.scale(58),
            self.scale(22),
            self.search,
        );
        place(
            c.prev,
            search_left + self.scale(280),
            top + self.scale(5),
            self.scale(30),
            self.scale(26),
            self.search,
        );
        place(
            c.next,
            search_left + self.scale(314),
            top + self.scale(5),
            self.scale(30),
            self.scale(26),
            self.search,
        );
        place(
            c.clear,
            search_left + self.scale(348),
            top + self.scale(5),
            self.scale(30),
            self.scale(26),
            self.search,
        );
        place(
            c.status,
            side_gap,
            height - foot + self.scale(3),
            (width - self.scale(220)).max(0),
            foot,
            true,
        );
        place(
            c.metrics,
            (width - self.scale(205)).max(0),
            height - foot + self.scale(3),
            self.scale(195),
            foot,
            true,
        );

        let pane = client(c.preview);
        let size = (pane.right, pane.bottom);
        if !self.raw && size != self.preview_size {
            let progress = self.page.progress();
            self.preview_size = size;
            self.reflow(progress);
        }
        repaint_window(self.window);
    }

    unsafe fn reflow(&mut self, progress: f64) {
        let result = Page::build(
            &self.renderer.write,
            &self.document,
            self.active
                .map(|i| self.state.documents[i].source_path.as_path()),
            &self.state.options,
            self.dip(self.preview_size.0),
            self.dip(self.preview_size.1),
        );
        match result {
            Ok(mut page) => {
                if self.search {
                    let query = get_text(self.controls.query);
                    if !query.is_empty() {
                        page.search(&query);
                    }
                }
                page.set_scroll(page.maximum_scroll() * progress.clamp(0., 1.) as f32);
                self.page = page;
                self.renderer.clear_document_images();
                self.update_scroll();
            }
            Err(error) => self.note(&format!("无法排版：{error}")),
        }
    }

    unsafe fn load_active(&mut self) {
        self.raw = false;
        self.editor_base.clear();
        self.raw_lines = raw::LineIndex::new("");
        set_text(self.controls.editor, "");
        self.state.active_id = self
            .active
            .map(|index| self.state.documents[index].id.clone());
        self.document = if let Some(index) = self.active {
            document::parse(&self.state.documents[index].content)
        } else {
            document::parse("# 未打开文档\n\n打开 Markdown 文件，或拖入文件与文件夹开始阅读。")
        };
        send(self.controls.heads, LB_RESETCONTENT, 0, 0);
        for item in &self.document.headings {
            if self.active.is_some() {
                list_add(
                    self.controls.heads,
                    &format!(
                        "{}{}",
                        "  ".repeat(item.level.saturating_sub(1) as usize),
                        item.title
                    ),
                );
            }
        }
        self.sync_controls();
        let progress = self
            .active
            .map(|i| self.state.documents[i].reading_progress)
            .unwrap_or(0.);
        self.arrange();
        self.reflow(progress);
    }

    unsafe fn refresh_list(&mut self, reveal: bool) {
        let mut filter = get_text(self.controls.filter).to_lowercase();
        if reveal
            && self.active.is_some_and(|i| {
                !self.state.documents[i]
                    .name
                    .to_lowercase()
                    .contains(&filter)
            })
        {
            filter.clear();
            set_text(self.controls.filter, "");
        }
        self.docs_visible.clear();
        send(self.controls.docs, LB_RESETCONTENT, 0, 0);
        for (index, entry) in self.state.documents.iter().enumerate() {
            if entry.name.to_lowercase().contains(&filter) {
                list_add(self.controls.docs, &entry.name);
                self.docs_visible.push(index);
            }
        }
        if let Some(row) = self
            .active
            .and_then(|i| self.docs_visible.iter().position(|pos| *pos == i))
        {
            send(self.controls.docs, LB_SETCURSEL, row, 0);
            let top = send(self.controls.docs, LB_GETTOPINDEX, 0, 0).max(0) as usize;
            let item = send(self.controls.docs, LB_GETITEMHEIGHT, 0, 0).max(1) as usize;
            let visible = client(self.controls.docs).bottom.max(0) as usize / item;
            if row < top || row >= top + visible {
                send(self.controls.docs, LB_SETTOPINDEX, row, 0);
            }
        }
    }

    unsafe fn import(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() || !self.confirm_leave() {
            return;
        }
        self.remember_progress();
        let current = self.active.map(|i| self.state.documents[i].id.clone());
        self.state.documents.retain(|entry| {
            std::fs::symlink_metadata(&entry.source_path)
                .map(|_| true)
                .unwrap_or_else(|error| {
                    error.kind() != std::io::ErrorKind::NotFound
                        || entry
                            .source_path
                            .ancestors()
                            .last()
                            .is_some_and(|root| !root.exists())
                })
        });
        self.active = current.and_then(|id| self.state.documents.iter().position(|i| i.id == id));
        let imported = files::import(paths, &self.state.documents);
        let count = imported.len();
        for mut item in imported {
            if let Some(position) = self
                .state
                .documents
                .iter()
                .position(|existing| files::path_key(&existing.source_path) == item.id)
            {
                item.reading_progress = self.state.documents[position].reading_progress;
                self.state.documents[position] = item;
                self.active = Some(position);
            } else {
                self.active = Some(self.state.documents.len());
                self.state.documents.push(item);
            }
        }
        if self.active.is_none() {
            self.active = self.state.documents.len().checked_sub(1);
        }
        self.refresh_list(true);
        self.load_active();
        self.note(&if count == 0 {
            "没有可打开的文档".to_string()
        } else {
            format!("已打开 {count} 份文档")
        });
        self.schedule_save();
    }

    unsafe fn toggle_raw(&mut self) {
        let Some(index) = self.active else {
            return;
        };
        if self.raw {
            if self.confirm_leave() {
                self.load_active();
            }
            return;
        }
        self.remember_progress();
        let prior = &self.state.documents[index];
        match files::open(&prior.source_path) {
            Ok(mut entry) => {
                entry.id = prior.id.clone();
                entry.reading_progress = prior.reading_progress;
                self.state.documents[index] = entry;
            }
            Err(error) => {
                platform::error_dialog(self.window, error);
                return;
            }
        }
        self.search = false;
        set_text(self.controls.query, "");
        self.editor_base = files::normalize_source(&self.state.documents[index].content);
        if self.editor_base.contains('\0') {
            platform::error_dialog(
                self.window,
                "源文件包含空字符，Raw 编辑器无法安全保存此文件",
            );
            return;
        }
        set_text(self.controls.editor, &self.editor_base);
        self.raw_lines = raw::LineIndex::new(&self.editor_base);
        raw::space_lines(self.controls.editor);
        self.raw_colors();
        send(self.controls.editor, EM_SETMODIFY, 0, 0);
        self.raw = true;
        let line = (self.editor_base.lines().count().saturating_sub(1) as f64
            * self.state.documents[index].reading_progress)
            .round() as usize;
        let source = self
            .editor_base
            .split_inclusive('\n')
            .take(line)
            .map(|part| part.encode_utf16().count())
            .sum::<usize>();
        send(self.controls.editor, EM_SETSEL, source, source as isize);
        self.sync_controls();
        self.arrange();
        SetFocus(self.controls.editor).ok();
        send(self.controls.editor, EM_SCROLLCARET, 0, 0);
        raw::highlight(self.controls.editor, self.state.options.is_dark);
        self.update_raw_cursor();
    }

    unsafe fn dirty(&self) -> bool {
        self.raw && send(self.controls.editor, EM_GETMODIFY, 0, 0) != 0
    }

    unsafe fn save_raw(&mut self) -> bool {
        let Some(index) = self.active.filter(|_| self.raw) else {
            return true;
        };
        let text = files::normalize_source(&get_text(self.controls.editor));
        if text == self.editor_base {
            send(self.controls.editor, EM_SETMODIFY, 0, 0);
            self.sync_controls();
            return true;
        }
        match files::save_document(&mut self.state.documents[index], &text) {
            Ok(()) => {
                self.editor_base = text;
                send(self.controls.editor, EM_SETMODIFY, 0, 0);
                self.sync_controls();
                self.note("已保存到原文件");
                self.schedule_save();
                true
            }
            Err(error) => {
                platform::error_dialog(self.window, error);
                false
            }
        }
    }

    unsafe fn confirm_leave(&mut self) -> bool {
        if !self.dirty()
            || files::normalize_source(&get_text(self.controls.editor)) == self.editor_base
        {
            return true;
        }
        let prompt = wide("当前文档有未保存的修改。\n\n是：保存　否：放弃　取消：继续编辑");
        match MessageBoxW(
            self.window,
            PCWSTR(prompt.as_ptr()),
            w!("保存修改？"),
            MB_YESNOCANCEL | MB_ICONQUESTION | MB_DEFBUTTON3,
        ) {
            IDYES => self.save_raw(),
            IDNO => true,
            _ => {
                SetFocus(self.controls.editor).ok();
                false
            }
        }
    }

    unsafe fn sync_controls(&self) {
        let c = &self.controls;
        let has = self.active.is_some();
        let dirty = self.dirty();
        let title = self
            .active
            .map(|index| self.state.documents[index].name.as_str())
            .unwrap_or("Folio");
        set_text(
            c.title,
            &format!("{title}{}", if dirty { " *" } else { "" }),
        );
        let directory = self
            .active
            .and_then(|index| self.state.documents[index].source_path.parent())
            .map(|p| p.to_string_lossy());
        let directory = directory.as_deref().unwrap_or("Markdown 文档");
        set_text(c.meta, directory.strip_prefix(r"\\?\").unwrap_or(directory));
        for (id, handle, _) in &c.toolbar {
            match *id {
                RAW => {
                    let _ = EnableWindow(*handle, has);
                    set_text(*handle, if self.raw { "预览" } else { "编辑" });
                    invalidate(*handle);
                }
                SAVE => {
                    let _ = EnableWindow(*handle, dirty);
                }
                OUTLINE => {
                    let _ = EnableWindow(*handle, has && !self.raw);
                }
                _ => {}
            }
        }
    }

    unsafe fn update_raw_cursor(&self) {
        let mut selection = CHARRANGE::default();
        send(
            self.controls.editor,
            EM_EXGETSEL,
            0,
            &mut selection as *mut _ as isize,
        );
        let (line, column) = self.raw_lines.cursor(selection.cpMax);
        set_text(
            self.controls.metrics,
            &format!("Raw · 第 {line} 行，第 {column} 列"),
        );
    }

    unsafe fn update_scroll(&self) {
        let total = self.page.total_height.ceil().max(1.) as i32;
        let scroll = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: total - 1,
            nPage: self.page.viewport_height.max(1.).ceil() as u32,
            nPos: self.page.scroll as i32,
            nTrackPos: 0,
        };
        SetScrollInfo(self.controls.preview, SB_VERT, &scroll, true);
        set_text(
            self.controls.metrics,
            &if self.active.is_some() {
                format!(
                    "{} 字    {:.0}%",
                    self.document.word_count,
                    self.page.progress() * 100.
                )
            } else {
                String::new()
            },
        );
        set_text(
            self.controls.matches,
            &if self.page.match_count() > 0 {
                format!(
                    "{} / {}",
                    self.page.match_index + 1,
                    self.page.match_count()
                )
            } else {
                "0 / 0".to_owned()
            },
        );
        invalidate(self.controls.preview);
    }

    unsafe fn scroll_to(&mut self, offset: f32) {
        self.page.set_scroll(offset);
        self.update_scroll();
        self.schedule_save();
    }
    fn remember_progress(&mut self) {
        if !self.raw {
            if let Some(i) = self.active {
                self.state.documents[i].reading_progress = self.page.progress();
            }
        }
    }
    unsafe fn note(&self, message: &str) {
        set_text(self.controls.status, message);
    }
    unsafe fn schedule_save(&self) {
        SetTimer(self.window, TIMER, 500, None);
    }

    unsafe fn toggle_search(&mut self, show: bool) {
        self.search = show;
        if !show {
            set_text(self.controls.query, "");
            self.page.search("");
            let _ = SetFocus(if self.raw {
                self.controls.editor
            } else {
                self.controls.preview
            });
        }
        self.arrange();
        if show {
            let _ = SetFocus(self.controls.query);
            send(self.controls.query, EM_SETSEL, 0, -1);
        }
    }

    unsafe fn search_raw(&mut self, backward: bool, advance: bool) {
        let query = get_text(self.controls.query);
        if query.is_empty() {
            set_text(self.controls.matches, "0 / 0");
            return;
        }
        let source = files::normalize_source(&get_text(self.controls.editor));
        let Ok(regex) = regex::RegexBuilder::new(&regex::escape(&query))
            .case_insensitive(true)
            .build()
        else {
            return;
        };
        let mut ranges = Vec::new();
        let mut previous = 0;
        let mut offset = 0;
        for hit in regex.find_iter(&source) {
            offset += source[previous..hit.start()].encode_utf16().count();
            let len = hit.as_str().encode_utf16().count();
            ranges.push((offset as i32, (offset + len) as i32));
            offset += len;
            previous = hit.end();
        }
        if ranges.is_empty() {
            set_text(self.controls.matches, "0 / 0");
            return;
        }
        let edit = self.controls.editor;
        let mut old = CHARRANGE::default();
        send(edit, EM_EXGETSEL, 0, &mut old as *mut _ as isize);
        let position = if !advance || self.raw_query != query {
            0
        } else if backward {
            ranges
                .iter()
                .rposition(|range| range.0 < old.cpMin)
                .unwrap_or(ranges.len() - 1)
        } else {
            ranges
                .iter()
                .position(|range| range.0 > old.cpMin)
                .unwrap_or(0)
        };
        let (minimum, maximum) = ranges[position];
        let sel = CHARRANGE {
            cpMin: minimum,
            cpMax: maximum,
        };
        send(edit, EM_EXSETSEL, 0, &sel as *const _ as isize);
        send(edit, EM_SCROLLCARET, 0, 0);
        self.update_raw_cursor();
        self.raw_query = query;
        set_text(
            self.controls.matches,
            &format!("{} / {}", position + 1, ranges.len()),
        );
    }

    unsafe fn command(&mut self, id: u16, event: u16) {
        match id {
            OPEN | FOLDER => match platform::choose_files(self.window, id == FOLDER) {
                Ok(paths) => self.import(&paths),
                Err(e) => platform::error_dialog(self.window, e),
            },
            RAW => self.toggle_raw(),
            SAVE => {
                self.save_raw();
            }
            FILTER if event as u32 == EN_CHANGE => self.refresh_list(false),
            DOCUMENTS if event as u32 == LBN_SELCHANGE => {
                let row = send(self.controls.docs, LB_GETCURSEL, 0, 0);
                if let Some(&index) = self.docs_visible.get(row as usize) {
                    if Some(index) != self.active {
                        if self.confirm_leave() {
                            self.remember_progress();
                            self.active = Some(index);
                            self.load_active();
                            self.schedule_save();
                        } else {
                            self.refresh_list(false);
                        }
                    }
                }
            }
            HEADINGS if event as u32 == LBN_SELCHANGE => {
                let row = send(self.controls.heads, LB_GETCURSEL, 0, 0);
                if let Some(heading) = self.document.headings.get(row as usize) {
                    self.page.jump(heading.block);
                    self.update_scroll();
                    self.schedule_save();
                }
            }
            EDITOR if self.raw => {
                if event as u32 == EN_CHANGE {
                    let text = files::normalize_source(&get_text(self.controls.editor));
                    self.raw_lines = raw::LineIndex::new(&text);
                    invalidate(self.controls.gutter);
                    SetTimer(self.window, RAW_COLOR_TIMER, 300, None);
                }
                self.sync_controls();
                self.update_raw_cursor();
            }
            LIBRARY => {
                self.state.sidebar_open = !self.state.sidebar_open;
                self.arrange();
                self.schedule_save();
            }
            OUTLINE if !self.raw => {
                self.state.outline_open = !self.state.outline_open;
                self.arrange();
                self.schedule_save();
            }
            SEARCH => self.toggle_search(!self.search),
            CLOSE_SEARCH => self.toggle_search(false),
            QUERY if event as u32 == EN_CHANGE => {
                if self.raw {
                    self.search_raw(false, false);
                } else {
                    self.page.search(&get_text(self.controls.query));
                    self.update_scroll();
                }
            }
            NEXT | PREVIOUS => {
                if self.raw {
                    self.search_raw(id == PREVIOUS, true);
                } else {
                    self.page.advance_match(id == PREVIOUS);
                    self.update_scroll();
                }
            }
            THEME => {
                self.state.options.is_dark = !self.state.options.is_dark;
                let was_modified = self.dirty();
                self.update_theme();
                if self.raw {
                    raw::highlight(self.controls.editor, self.state.options.is_dark);
                }
                send(self.controls.editor, EM_SETMODIFY, was_modified as usize, 0);
                if !self.raw {
                    self.reflow(self.page.progress());
                }
                self.sync_controls();
                self.schedule_save();
            }
            SETTINGS => self.popup(
                SETTINGS,
                &[
                    (FONT_INCREASE, "增大字号"),
                    (FONT_DECREASE, "减小字号"),
                    (WIDTH_AUTO, "自动正文宽度"),
                    (WIDTH_NARROW, "缩窄正文"),
                    (WIDTH_WIDER, "加宽正文"),
                    (SPACING_INCREASE, "增大行距"),
                    (SPACING_DECREASE, "减小行距"),
                    (FONT_SANS, "无衬线"),
                    (FONT_SERIF, "衬线"),
                    (FONT_MONO, "等宽"),
                ],
            ),
            MORE => self.popup(
                MORE,
                &[
                    (FOCUS, "专注模式  F11"),
                    (REFRESH, "重新读取  Ctrl+R"),
                    (COPY, "复制正文"),
                    (PRINT, "打印…  Ctrl+P"),
                ],
            ),
            REFRESH => {
                if let Some(index) = self.active {
                    let path = self.state.documents[index].source_path.clone();
                    self.import(&[path]);
                }
            }
            FOCUS => {
                self.focus = !self.focus;
                self.arrange();
            }
            COPY => {
                if self.raw {
                    send(self.controls.editor, WM_COPY, 0, 0);
                } else if self.active.is_some() {
                    let previous = self.page.selection;
                    self.page.select_all();
                    if let Some(text) = self.page.selected() {
                        if let Err(error) = platform::clipboard(self.window, &text) {
                            platform::error_dialog(self.window, error);
                        }
                    }
                    self.page.selection = previous;
                }
            }
            PRINT => self.print_document(),
            FONT_INCREASE..=FONT_MONO => {
                let options = &mut self.state.options;
                match id {
                    FONT_INCREASE => options.font_size += 1.,
                    FONT_DECREASE => options.font_size -= 1.,
                    WIDTH_AUTO => options.auto_width = !options.auto_width,
                    WIDTH_NARROW => {
                        options.auto_width = false;
                        options.content_width -= 40.;
                    }
                    WIDTH_WIDER => {
                        options.auto_width = false;
                        options.content_width += 40.;
                    }
                    SPACING_INCREASE => options.line_height += 0.1,
                    SPACING_DECREASE => options.line_height -= 0.1,
                    FONT_SANS => options.font_family = "sans".into(),
                    FONT_SERIF => options.font_family = "serif".into(),
                    FONT_MONO => options.font_family = "mono".into(),
                    _ => {}
                }
                options.normalize();
                if !self.raw {
                    self.reflow(self.page.progress());
                }
                self.schedule_save();
            }
            _ => {}
        }
    }

    unsafe fn popup(&mut self, anchor: u16, items: &[(u16, &str)]) {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        for (id, title) in items {
            let _ = AppendMenuW(menu, MF_STRING, *id as usize, PCWSTR(wide(title).as_ptr()));
        }
        let mut area = RECT::default();
        if let Some((_, hwnd, _)) = self
            .controls
            .toolbar
            .iter()
            .find(|(id, _, _)| *id == anchor)
        {
            let _ = GetWindowRect(*hwnd, &mut area);
        }
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTALIGN,
            area.right,
            area.bottom,
            0,
            self.window,
            None,
        )
        .0;
        let _ = DestroyMenu(menu);
        if chosen > 0 {
            self.command(chosen as u16, 0);
        }
    }

    unsafe fn print_document(&mut self) {
        let Some(index) = self.active.filter(|_| !self.raw) else {
            return;
        };
        if let Err(error) = platform::print_page(
            self.window,
            &self.document,
            &self.state.documents[index].source_path,
            &self.state.options,
        ) {
            platform::error_dialog(self.window, error);
        }
    }

    unsafe fn advance_measurement(&mut self) {
        let Some(run) = self.measurement.as_mut() else {
            return;
        };
        let Some(step) = run.next() else {
            if let Err(error) = run.finish() {
                platform::error_dialog(self.window, error);
            }
            self.measurement = None;
            let _ = PostMessageW(self.window, WM_CLOSE, WPARAM(0), LPARAM(0));
            return;
        };
        if step.open {
            self.import(&step.paths);
        } else {
            for path in step.paths {
                let key = files::path_key(&path);
                if let Some(index) = self
                    .state
                    .documents
                    .iter()
                    .position(|entry| files::path_key(&entry.source_path) == key)
                {
                    self.remember_progress();
                    self.active = Some(index);
                    self.refresh_list(true);
                    self.load_active();
                }
            }
        }
    }

    unsafe fn link(&mut self, target: &str) {
        if let Some(anchor) = target.strip_prefix('#') {
            let anchor = percent_decode_str(anchor).decode_utf8_lossy();
            if let Some(heading) = self
                .document
                .headings
                .iter()
                .find(|heading| anchor == heading.id)
            {
                self.page.jump(heading.block);
                self.update_scroll();
                self.schedule_save();
            }
            return;
        }
        let scheme = target
            .split_once(':')
            .map(|pair| pair.0.to_ascii_lowercase());
        if matches!(scheme.as_deref(), Some("http" | "https" | "mailto")) {
            ShellExecuteW(
                self.window,
                w!("open"),
                PCWSTR(wide(target).as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        } else if scheme.is_none() {
            let Some(index) = self.active else {
                return;
            };
            let relative = percent_decode_str(target.split(['#', '?']).next().unwrap_or_default())
                .decode_utf8_lossy();
            let path = Path::new(relative.as_ref());
            if path.is_absolute() {
                return;
            }
            if let Some(path) = self.state.documents[index]
                .source_path
                .parent()
                .map(|base| base.join(path))
                .filter(|file| files::is_markdown(file))
            {
                self.import(&[path]);
            }
        }
    }

    unsafe fn key(&mut self, message: &MSG) -> bool {
        if message.message != WM_KEYDOWN {
            return false;
        }
        let key = message.wParam.0 as u16;
        let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        if key == VK_TAB.0 && !control && message.hwnd != self.controls.editor {
            if let Ok(next) = GetNextDlgTabItem(self.window, message.hwnd, shift) {
                let _ = SetFocus(next);
                return true;
            }
        }
        let matches = match key {
            0x4f if control => {
                if shift {
                    FOLDER
                } else {
                    OPEN
                }
            }
            0x53 if control && self.raw => SAVE,
            0x46 if control => SEARCH,
            0x52 if control => REFRESH,
            0x50 if control => PRINT,
            0xbc if control => SETTINGS,
            0xbb | 0x6b if control => FONT_INCREASE,
            0xbd | 0x6d if control => FONT_DECREASE,
            0x72 => {
                if shift {
                    PREVIOUS
                } else {
                    NEXT
                }
            }
            0x7a => FOCUS,
            0x1b if self.search => CLOSE_SEARCH,
            0x0d if message.hwnd == self.controls.query => {
                if shift {
                    PREVIOUS
                } else {
                    NEXT
                }
            }
            _ => 0,
        };
        if matches != 0 {
            self.command(matches, 0);
            return true;
        }
        if message.hwnd != self.controls.preview || self.active.is_none() {
            return false;
        }
        match key {
            0x41 if control => {
                self.page.select_all();
                invalidate(self.controls.preview);
            }
            0x43 if control => {
                if let Some(text) = self.page.selected() {
                    let _ = platform::clipboard(self.window, &text);
                }
            }
            0x21 => self.scroll_to(self.page.scroll - self.page.viewport_height * 0.9),
            0x22 | 0x20 => self.scroll_to(self.page.scroll + self.page.viewport_height * 0.9),
            0x24 => self.scroll_to(0.),
            0x23 => self.scroll_to(self.page.maximum_scroll()),
            0x26 => self.scroll_to(self.page.scroll - 36.),
            0x28 => self.scroll_to(self.page.scroll + 36.),
            _ => return false,
        }
        true
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for handle in [
                self.font.0,
                self.filter_font.0,
                self.edit_font.0,
                self.gutter_font.0,
                self.surface.0,
                self.rail_surface.0,
            ] {
                if !handle.is_null() {
                    let _ = DeleteObject(HGDIOBJ(handle));
                }
            }
        }
    }
}

unsafe fn send(window: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    SendMessageW(window, message, WPARAM(wparam), LPARAM(lparam)).0
}

unsafe fn child(
    parent: HWND,
    class: PCWSTR,
    label: &str,
    styles: WINDOW_STYLE,
    id: u16,
) -> Result<HWND> {
    CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        PCWSTR(wide(label).as_ptr()),
        WS_CHILD | styles,
        0,
        0,
        1,
        1,
        parent,
        HMENU(id as usize as *mut c_void),
        HINSTANCE(GetModuleHandleW(None)?.0),
        None,
    )
}

unsafe fn list_add(window: HWND, text: &str) {
    send(window, LB_ADDSTRING, 0, wide(text).as_ptr() as isize);
}
unsafe fn get_text(window: HWND) -> String {
    let len = GetWindowTextLengthW(window).max(0) as usize;
    let mut text = vec![0; len + 1];
    let read = GetWindowTextW(window, &mut text).max(0) as usize;
    String::from_utf16_lossy(&text[..read])
}
unsafe fn set_text(window: HWND, text: &str) {
    let _ = SetWindowTextW(window, PCWSTR(wide(text).as_ptr()));
}
unsafe fn place(window: HWND, x: i32, y: i32, w: i32, h: i32, visible: bool) {
    if visible {
        let _ = MoveWindow(window, x, y, w.max(1), h.max(1), true);
    }
    show(window, visible);
}
unsafe fn show(window: HWND, visible: bool) {
    let _ = ShowWindow(window, if visible { SW_SHOWNA } else { SW_HIDE });
}
unsafe fn client(window: HWND) -> RECT {
    let mut rect = RECT::default();
    let _ = GetClientRect(window, &mut rect);
    rect
}
unsafe fn invalidate(window: HWND) {
    let _ = InvalidateRect(window, None, false);
}

unsafe fn repaint_window(window: HWND) {
    let _ = RedrawWindow(
        window,
        None,
        None,
        RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
    );
}
fn point(lparam: LPARAM) -> (i32, i32) {
    (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32)
}

fn win_color(rgb: u32) -> COLORREF {
    COLORREF((rgb & 0xff00) | ((rgb & 0xff) << 16) | ((rgb >> 16) & 0xff))
}

unsafe extern "system" fn filter_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    if message == WM_PAINT && GetFocus() != hwnd && GetWindowTextLengthW(hwnd) == 0 {
        let dark = GetParent(hwnd)
            .is_ok_and(|parent| !GetPropW(parent, w!("FolioDarkTheme")).is_invalid());
        let tone = icons::Tone::new(dark);
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        let mut bounds = client(hwnd);
        SetDCBrushColor(dc, win_color(tone.canvas));
        FillRect(dc, &bounds, HBRUSH(GetStockObject(DC_BRUSH).0));
        let font = HGDIOBJ(send(hwnd, WM_GETFONT, 0, 0) as *mut c_void);
        let previous = SelectObject(dc, font);
        SetTextColor(dc, win_color(tone.muted));
        SetBkMode(dc, TRANSPARENT);
        let mut text: Vec<_> = FILTER_HINT.encode_utf16().collect();
        DrawTextW(
            dc,
            &mut text,
            &mut bounds,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        let _ = EndPaint(hwnd, &paint);
        return LRESULT(0);
    }
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(filter_proc), id);
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if matches!(message, WM_SETFOCUS | WM_KILLFOCUS) {
        invalidate(hwnd);
    }
    result
}

unsafe extern "system" fn gutter_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if pointer == 0 {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let cell = &*(pointer as *const RefCell<App>);
    if message == WM_ERASEBKGND {
        return LRESULT(1);
    }
    if message == WM_PAINT {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        if let Ok(app) = cell.try_borrow() {
            let bounds = client(hwnd);
            let tone = icons::Tone::new(app.state.options.is_dark);
            FillRect(dc, &bounds, app.surface);
            let font = SelectObject(dc, app.gutter_font);
            SetTextAlign(dc, TA_RIGHT | TA_BASELINE);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, win_color(tone.muted));
            let right = bounds.right - app.scale(9);
            let editor = app.controls.editor;
            let mut text_area = RECT::default();
            send(editor, EM_GETRECT, 0, &mut text_area as *mut _ as isize);
            IntersectClipRect(dc, 0, text_area.top, bounds.right, text_area.bottom);
            let limit = (bounds.bottom / app.scale(18).max(1)).max(1) as usize + 2;
            for (baseline, line) in app.raw_lines.visible_rows(editor, limit) {
                let number: Vec<_> = line.to_string().encode_utf16().collect();
                let _ = TextOutW(dc, right, baseline, &number);
            }
            SelectObject(dc, font);
        }
        let _ = EndPaint(hwnd, &paint);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam.0 as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    if message == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if pointer == 0 {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let cell = &*(pointer as *const RefCell<App>);
    let Ok(mut app) = cell.try_borrow_mut() else {
        if message == WM_CLOSE {
            return LRESULT(0);
        }
        if matches!(
            message,
            WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX
        ) {
            let dark = !GetPropW(hwnd, w!("FolioDarkTheme")).is_invalid();
            let tone = icons::Tone::new(dark);
            let background = win_color(if message == WM_CTLCOLORLISTBOX {
                tone.rail
            } else {
                tone.canvas
            });
            let dc = HDC(wparam.0 as *mut c_void);
            SetTextColor(dc, win_color(tone.text));
            SetBkColor(dc, background);
            SetDCBrushColor(dc, background);
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_CREATE => {
            app.window = hwnd;
            if let Err(error) = app.create_controls(pointer) {
                platform::error_dialog(hwnd, error);
                return LRESULT(-1);
            }
        }
        WM_GETMINMAXINFO => {
            let limits = &mut *(lparam.0 as *mut MINMAXINFO);
            limits.ptMinTrackSize.x = app.scale(720);
            limits.ptMinTrackSize.y = app.scale(520);
        }
        WM_SIZE => {
            if wparam.0 as u32 != SIZE_MINIMIZED && !app.controls.preview.0.is_null() {
                app.arrange();
                app.state.maximized = IsZoomed(hwnd).as_bool();
                if !app.state.maximized {
                    let mut outer = RECT::default();
                    if GetWindowRect(hwnd, &mut outer).is_ok() {
                        app.state.width = app.dip(outer.right - outer.left);
                        app.state.height = app.dip(outer.bottom - outer.top);
                    }
                }
                app.schedule_save();
            }
        }
        WM_DPICHANGED => {
            app.dpi = (wparam.0 & 0xffff) as u32;
            let frame = *(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                frame.left,
                frame.top,
                frame.right - frame.left,
                frame.bottom - frame.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            app.update_fonts();
            app.preview_size = (-1, -1);
            app.arrange();
        }
        WM_COMMAND => app.command(wparam.0 as u16, (wparam.0 >> 16) as u16),
        WM_DRAWITEM => {
            let info = &*(lparam.0 as *const DRAWITEMSTRUCT);
            let tone = icons::Tone::new(app.state.options.is_dark);
            match info.CtlID as u16 {
                OPEN | FOLDER => {
                    let (icon, label) = if info.CtlID as u16 == OPEN {
                        ("folder-open", "打开")
                    } else {
                        ("folder", "导入")
                    };
                    let _ = app.icons.action(info, icon, label, &tone, app.dpi);
                    return LRESULT(1);
                }
                DOCUMENTS | HEADINGS => {
                    let index = info.itemID as usize;
                    let label = if info.CtlID as u16 == DOCUMENTS {
                        app.docs_visible
                            .get(index)
                            .and_then(|index| app.state.documents.get(*index))
                            .map(|item| item.name.clone())
                    } else {
                        app.document.headings.get(index).map(|item| {
                            format!(
                                "{}{}",
                                "   ".repeat(item.level.saturating_sub(1) as usize),
                                item.title
                            )
                        })
                    };
                    let _ = app.icons.list_row(
                        info,
                        label.as_deref().unwrap_or_default(),
                        &tone,
                        app.dpi,
                    );
                    return LRESULT(1);
                }
                _ => {}
            }
            let (name, pressed) = match info.CtlID as u16 {
                LIBRARY => ("panel-left", app.state.sidebar_open && !app.focus),
                RAW => (if app.raw { "eye" } else { "pencil" }, app.raw),
                SAVE => ("save", false),
                SEARCH => ("search", app.search),
                OUTLINE => ("list", app.state.outline_open && !app.focus && !app.raw),
                THEME => ("moon", app.state.options.is_dark),
                SETTINGS => ("settings", false),
                MORE => ("ellipsis", false),
                _ => {
                    drop(app);
                    return DefWindowProcW(hwnd, message, wparam, lparam);
                }
            };
            if app
                .icons
                .draw(name, info, app.state.options.is_dark, pressed, app.dpi)
                .is_ok()
            {
                return LRESULT(1);
            }
        }
        WM_DROPFILES => app.import(&platform::drag_paths(HDROP(wparam.0 as *mut c_void))),
        performance::WM_MEASUREMENT => app.advance_measurement(),
        WM_LAUNCH => {
            let queued = app
                .queue
                .lock()
                .map(|mut pending| std::mem::take(&mut *pending))
                .unwrap_or_default();
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else if !IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            let _ = SetForegroundWindow(hwnd);
            for files in queued {
                app.import(&files);
            }
        }
        WM_TIMER if wparam.0 == TIMER => {
            let _ = KillTimer(hwnd, TIMER);
            app.remember_progress();
            if let Err(error) = app.store.save(&app.state, false) {
                app.note(&format!("保存阅读状态失败：{error}"));
            }
        }
        WM_TIMER if wparam.0 == RAW_COLOR_TIMER => {
            let _ = KillTimer(hwnd, RAW_COLOR_TIMER);
            if app.raw {
                raw::highlight(app.controls.editor, app.state.options.is_dark);
            }
        }
        WM_CLOSE => {
            if !app.confirm_leave() {
                return LRESULT(0);
            }
            let _ = KillTimer(hwnd, TIMER);
            if let Err(error) = app.store.save(&app.state, true) {
                platform::error_dialog(hwnd, error);
                return LRESULT(0);
            }
            drop(app);
            let _ = DestroyWindow(hwnd);
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let dc = HDC(wparam.0 as *mut c_void);
            let child = HWND(lparam.0 as *mut c_void);
            let tone = icons::Tone::new(app.state.options.is_dark);
            let muted = [
                app.controls.status,
                app.controls.meta,
                app.controls.metrics,
                app.controls.matches,
            ]
            .contains(&child);
            SetTextColor(dc, win_color(if muted { tone.muted } else { tone.text }));
            let rail = message == WM_CTLCOLORLISTBOX || child == app.controls.heads_title;
            SetBkColor(dc, win_color(if rail { tone.rail } else { tone.canvas }));
            return LRESULT(if rail {
                app.rail_surface.0
            } else {
                app.surface.0
            } as isize);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut paint);
            let tone = icons::Tone::new(app.state.options.is_dark);
            if app
                .icons
                .shell(
                    paint.hdc,
                    &client(hwnd),
                    &app.shell,
                    &tone,
                    app.dpi as f32 / 96.,
                )
                .is_err()
            {
                FillRect(paint.hdc, &paint.rcPaint, app.surface);
            }
            let _ = EndPaint(hwnd, &paint);
        }
        _ => {
            // The default handler can synchronously send WM_CLOSE for SC_CLOSE.
            drop(app);
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
    }
    LRESULT(0)
}

unsafe extern "system" fn preview_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if pointer == 0 {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    let cell = &*(pointer as *const RefCell<App>);
    let Ok(mut app) = cell.try_borrow_mut() else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut paint);
            let bounds = client(hwnd);
            let pixels = (bounds.right.max(1) as u32, bounds.bottom.max(1) as u32);
            let app = &mut *app;
            let frame =
                app.renderer
                    .draw(hwnd, pixels, app.dpi as f32, &app.page, &app.state.options);
            if frame.is_err() {
                FillRect(paint.hdc, &bounds, app.surface);
            }
            let _ = EndPaint(hwnd, &paint);
            if frame.is_ok() {
                if let (Some(run), Some(index)) = (app.measurement.as_mut(), app.active) {
                    let current = &app.state.documents[index].source_path;
                    if run.presented(current, app.document.blocks.len()) {
                        let _ = PostMessageW(
                            app.window,
                            performance::WM_MEASUREMENT,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DROPFILES => app.import(&platform::drag_paths(HDROP(wparam.0 as *mut c_void))),
        WM_VSCROLL => {
            let command = SCROLLBAR_COMMAND(wparam.0 as u16 as i32);
            let prior = app.page.scroll;
            let offset = match command {
                SB_LINEUP => prior - 40.,
                SB_LINEDOWN => prior + 40.,
                SB_PAGEUP => prior - app.page.viewport_height * 0.9,
                SB_PAGEDOWN => prior + app.page.viewport_height * 0.9,
                SB_TOP => 0.,
                SB_BOTTOM => app.page.maximum_scroll(),
                SB_THUMBTRACK | SB_THUMBPOSITION => {
                    let mut info = SCROLLINFO {
                        cbSize: size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_TRACKPOS,
                        ..Default::default()
                    };
                    let _ = GetScrollInfo(hwnd, SB_VERT, &mut info);
                    info.nTrackPos as f32
                }
                _ => prior,
            };
            app.scroll_to(offset);
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let ticks = (wparam.0 >> 16) as i16 as f32 / 120.;
            if message == WM_MOUSEHWHEEL || GetKeyState(VK_SHIFT.0 as i32) < 0 {
                let (x, y) = point(lparam);
                let mut local = POINT { x, y };
                let _ = ScreenToClient(hwnd, &mut local);
                let y = app.dip(local.y);
                app.page.shift_horizontal(
                    y,
                    if message == WM_MOUSEHWHEEL {
                        ticks * 96.
                    } else {
                        -ticks * 96.
                    },
                );
                invalidate(hwnd);
            } else {
                let offset = app.page.scroll - ticks * 92.;
                app.scroll_to(offset);
            }
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(hwnd);
            let (x, y) = point(lparam);
            match app.page.hit(app.dip(x), app.dip(y), true) {
                Hit::Text(position) => {
                    app.page.selection = Some((position, position));
                    app.dragging = true;
                    SetCapture(hwnd);
                }
                Hit::Link(link, position) => {
                    app.pending_link = Some(link);
                    app.page.selection = Some((position, position));
                    app.dragging = true;
                    SetCapture(hwnd);
                }
                Hit::Horizontal(block, position) => {
                    app.page.horizontal_scroll(block, position);
                    app.horizontal_drag = Some(block);
                    SetCapture(hwnd);
                }
                Hit::Code(code) => {
                    let _ = platform::clipboard(app.window, &code);
                    app.note("代码已复制");
                }
                Hit::None => {
                    app.page.selection = None;
                }
            }
            invalidate(hwnd);
        }
        WM_MOUSEMOVE if app.dragging || app.horizontal_drag.is_some() => {
            let (x, y) = point(lparam);
            let (x, y) = (app.dip(x), app.dip(y));
            if app.horizontal_drag.is_some() {
                if let Hit::Horizontal(block, pos) = app.page.hit(x, y, true) {
                    app.page.horizontal_scroll(block, pos);
                }
            } else if let Some((start, _)) = app.page.selection {
                if y < 0. || y > app.page.viewport_height {
                    let offset = app.page.scroll + if y < 0. { -25. } else { 25. };
                    app.scroll_to(offset);
                }
                if let Hit::Text(position) = app.page.hit(x, y, false) {
                    if position != start {
                        app.pending_link = None;
                    }
                    app.page.selection = Some((start, position));
                }
            }
            invalidate(hwnd);
        }
        WM_LBUTTONUP if app.dragging || app.horizontal_drag.is_some() => {
            app.dragging = false;
            app.horizontal_drag = None;
            let _ = ReleaseCapture();
            if let Some(target) = app.pending_link.take() {
                app.link(&target);
            }
        }
        WM_CAPTURECHANGED => {
            app.dragging = false;
            app.horizontal_drag = None;
            app.pending_link = None;
        }
        _ => return DefWindowProcW(hwnd, message, wparam, lparam),
    }
    LRESULT(0)
}

fn run(process_started: Instant) -> Result<()> {
    let store =
        Store::new().map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let measurement = performance::Measurement::parse(&args, &store.root, process_started)
        .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
    let paths = if measurement.is_none() {
        platform::command_line_paths()
    } else {
        Vec::new()
    };
    let Election::Primary(_instance, pipe) = platform::elect(&store.root, &paths)? else {
        return Ok(());
    };
    let cell = Box::new(RefCell::new(App::new(store)?));
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let text_cursor = LoadCursorW(None, IDC_IBEAM)?;
        let icon = LoadIconW(instance, PCWSTR(1 as *const u16))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))?;
        let class = WNDCLASSW {
            hInstance: instance,
            lpszClassName: WINDOW,
            hCursor: cursor,
            hIcon: icon,
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        let pane = WNDCLASSW {
            hInstance: instance,
            lpszClassName: PREVIEW,
            hCursor: text_cursor,
            lpfnWndProc: Some(preview_proc),
            ..Default::default()
        };
        let gutter = WNDCLASSW {
            hInstance: instance,
            lpszClassName: GUTTER,
            hCursor: cursor,
            lpfnWndProc: Some(gutter_proc),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 || RegisterClassW(&pane) == 0 || RegisterClassW(&gutter) == 0
        {
            return Err(windows::core::Error::from_win32());
        }
        let (width, height, maximum) = {
            let app = cell.borrow();
            (
                app.state.width.clamp(720., 3200.),
                app.state.height.clamp(520., 2400.),
                app.state.maximized,
            )
        };
        let scale = GetDpiForSystem().max(96) as f32 / 96.;
        let mut area = RECT::default();
        let placement = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut area as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok();
        let width = if placement {
            ((width * scale) as i32).min(area.right - area.left)
        } else {
            (width * scale) as i32
        };
        let height = if placement {
            ((height * scale) as i32).min(area.bottom - area.top)
        } else {
            (height * scale) as i32
        };
        let x = if placement {
            (area.left + area.right - width) / 2
        } else {
            CW_USEDEFAULT
        };
        let y = if placement {
            (area.top + area.bottom - height) / 2
        } else {
            CW_USEDEFAULT
        };
        let hwnd = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            WINDOW,
            w!("Folio"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            x,
            y,
            width,
            height,
            None,
            None,
            instance,
            Some((&*cell as *const RefCell<App>).cast()),
        )?;
        platform::listen(pipe, hwnd, Arc::clone(&cell.borrow().queue))?;
        let show = if maximum {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOWNORMAL
        };
        let _ = ShowWindow(hwnd, show);
        // The first call can inherit a hidden show state from the launcher.
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, show);
        }
        {
            let mut app = cell.borrow_mut();
            if !paths.is_empty() {
                app.import(&paths);
            }
            app.measurement = measurement;
            let _ = SetFocus(app.controls.preview);
        }
        if cell.borrow().measurement.is_some() {
            let _ = PostMessageW(hwnd, performance::WM_MEASUREMENT, WPARAM(0), LPARAM(0));
        }
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            let handled = cell.borrow_mut().key(&message);
            if !handled {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            if let Ok(app) = cell.try_borrow() {
                if app.raw
                    && [WM_KEYUP, WM_LBUTTONUP, WM_MOUSEWHEEL].contains(&message.message)
                    && message.hwnd == app.controls.editor
                {
                    app.update_raw_cursor();
                }
            }
        }
        CoUninitialize();
    }
    Ok(())
}

fn main() {
    if let Err(error) = run(Instant::now()) {
        platform::error_dialog(HWND::default(), error);
        std::process::exit(1);
    }
}
