use crate::document::{plain, Body, Document, Span, Style};
use crate::files::{self, Options};
use comrak::nodes::TableAlignment;
use image::{ImageReader, Limits, RgbaImage};
use inkjet::{
    constants::HIGHLIGHT_NAMES, tree_sitter_highlight::HighlightEvent, Highlighter, Language,
};
use percent_encoding::percent_decode_str;
use std::collections::{HashMap, VecDeque};
use std::io::{BufReader, Cursor};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use windows::core::{w, Result};
use windows::Win32::Foundation::{BOOL, HWND};
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn point(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}
fn rgb(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (hex >> 16 & 255) as f32 / 255.,
        g: (hex >> 8 & 255) as f32 / 255.,
        b: (hex & 255) as f32 / 255.,
        a: 1.,
    }
}

const IMAGE_SOURCE_LIMIT: usize = 128;
const IMAGE_INPUT_LIMIT: usize = 16 * 1024 * 1024;
const IMAGE_DIMENSION_LIMIT: u32 = 4096;
const IMAGE_BITMAP_DIMENSION: u32 = 1024;
const IMAGE_CACHE_LIMIT: usize = 8;

pub struct Renderer {
    factory: ID2D1Factory,
    pub write: IDWriteFactory,
    surface: Option<ID2D1HwndRenderTarget>,
    palette: HashMap<u32, ID2D1SolidColorBrush>,
    images: VecDeque<(Arc<PathBuf>, Option<ID2D1Bitmap>)>,
}

impl Renderer {
    pub fn new() -> Result<Self> {
        unsafe {
            Ok(Self {
                factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                surface: None,
                palette: HashMap::new(),
                images: VecDeque::new(),
            })
        }
    }

    fn surface(
        &mut self,
        hwnd: HWND,
        width: u32,
        height: u32,
        dpi: f32,
    ) -> Result<ID2D1HwndRenderTarget> {
        let size = D2D_SIZE_U {
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(surface) = &self.surface {
            unsafe {
                if surface.GetPixelSize() != size {
                    surface.Resize(&size)?;
                }
                surface.SetDpi(dpi, dpi);
            }
            return Ok(surface.clone());
        }
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            dpiX: dpi,
            dpiY: dpi,
            ..Default::default()
        };
        let window = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd,
            pixelSize: size,
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };
        let surface = unsafe { self.factory.CreateHwndRenderTarget(&properties, &window)? };
        self.surface = Some(surface.clone());
        Ok(surface)
    }

    fn brush(&mut self, surface: &ID2D1RenderTarget, color: u32) -> Result<ID2D1SolidColorBrush> {
        if let Some(value) = self.palette.get(&color) {
            return Ok(value.clone());
        }
        let brush = unsafe { surface.CreateSolidColorBrush(&rgb(color), None)? };
        self.palette.insert(color, brush.clone());
        Ok(brush)
    }

    pub fn clear_document_images(&mut self) {
        self.images.clear();
    }

    fn image(&mut self, surface: &ID2D1RenderTarget, path: &Arc<PathBuf>) -> Option<ID2D1Bitmap> {
        if let Some(index) = self.images.iter().position(|(key, _)| key == path) {
            let item = self.images.remove(index).expect("cached image index");
            let image = item.1.clone();
            self.images.push_back(item);
            return image;
        }
        if self.images.len() == IMAGE_CACHE_LIMIT {
            self.images.pop_front();
        }
        let image = load_image(path).and_then(|data| bitmap(surface, data).ok());
        self.images.push_back((Arc::clone(path), image.clone()));
        image
    }

    pub fn draw(
        &mut self,
        hwnd: HWND,
        pixels: (u32, u32),
        dpi: f32,
        page: &Page,
        options: &Options,
    ) -> Result<()> {
        let surface = self.surface(hwnd, pixels.0, pixels.1, dpi)?;
        let status = self.draw_target(&surface, page, options);
        if status.is_err() {
            self.surface = None;
            self.palette.clear();
            self.images.clear();
        }
        status
    }

    pub fn printer_target(&self, dpi_x: f32, dpi_y: f32) -> Result<ID2D1DCRenderTarget> {
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi_x,
            dpiY: dpi_y,
            ..Default::default()
        };
        unsafe { self.factory.CreateDCRenderTarget(&properties) }
    }

    pub fn draw_target(
        &mut self,
        surface: &ID2D1RenderTarget,
        page: &Page,
        options: &Options,
    ) -> Result<()> {
        let foreground = self.brush(surface, if options.is_dark { 0xeceef2 } else { 0x1e2024 })?;
        let accent = self.brush(&surface, if options.is_dark { 0x90b8fa } else { 0x235b9d })?;
        let muted = self.brush(surface, if options.is_dark { 0xa0a7b4 } else { 0x656f7e })?;
        let border = self.brush(surface, if options.is_dark { 0x343943 } else { 0xe4e9f0 })?;
        let selected = self.brush(&surface, if options.is_dark { 0x294671 } else { 0xd4e4fa })?;
        let matching = self.brush(&surface, if options.is_dark { 0x6a512c } else { 0xffecaf })?;
        let inline_code = self.brush(surface, if options.is_dark { 0x2a3037 } else { 0xf1f4f8 })?;
        let canvas = rgb(if options.is_dark { 0x1e2124 } else { 0xffffff });
        unsafe {
            surface.BeginDraw();
            surface.Clear(Some(&canvas));
        }
        let operation = (|| {
            let top = page.scroll;
            let bottom = top + page.viewport_height;
            let start = page
                .groups
                .partition_point(|block| block.y + block.height < top);
            for group in &page.groups[start..] {
                if group.y > bottom {
                    break;
                }
                let y = group.y - top;
                let clip = rect(group.x, y, group.view_width.max(1.), group.height);
                unsafe {
                    surface.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                }
                let result: Result<()> = (|| {
                    let offset = page
                        .horizontal
                        .get(&group.block)
                        .copied()
                        .unwrap_or_default();
                    let scroll_x = offset.min(group.maximum_horizontal());
                    for item in &group.items {
                        match item {
                            Item::Fill(x, fy, width, height, kind) => {
                                let color = match (*kind, options.is_dark) {
                                    (0, false) => 0xf4f6f9,
                                    (0, true) => 0x282c32,
                                    (1, false) => 0xe2e6ec,
                                    (1, true) => 0x3b424d,
                                    (2, false) => 0xf8fafc,
                                    (2, true) => 0x23272d,
                                    (_, false) => 0xd3dbe7,
                                    (_, true) => 0x404a58,
                                };
                                let brush = self.brush(&surface, color)?;
                                let area = rect(group.x + x - scroll_x, y + fy, *width, *height);
                                unsafe {
                                    if group.code.is_some() && *kind == 0 {
                                        let card = D2D1_ROUNDED_RECT {
                                            rect: area,
                                            radiusX: 7.,
                                            radiusY: 7.,
                                        };
                                        surface.FillRoundedRectangle(&card, &brush);
                                        surface.DrawRoundedRectangle(&card, &border, 1., None);
                                    } else {
                                        surface.FillRectangle(&area, &brush);
                                    }
                                }
                            }
                            Item::Text(run) => {
                                let origin = point(
                                    group.x + run.x - if run.fixed { 0. } else { scroll_x },
                                    y + run.y,
                                );
                                if origin.y + run.height < 0. || origin.y > page.viewport_height {
                                    continue;
                                }
                                for range in &run.code_ranges {
                                    highlight(surface, run, origin, range.clone(), &inline_code);
                                }
                                if let Some(range) = page.matched_range() {
                                    highlight(&surface, run, origin, range, &matching);
                                }
                                if let Some(range) = page.selection_range() {
                                    highlight(&surface, run, origin, range, &selected);
                                }
                                for (range, color) in &run.effects {
                                    let brush = if *color == 0 {
                                        accent.clone()
                                    } else {
                                        self.brush(&surface, *color)?
                                    };
                                    unsafe {
                                        let _ = run.layout.SetDrawingEffect(&brush, *range);
                                    }
                                }
                                let brush = if run.action {
                                    &accent
                                } else if run.fixed {
                                    &muted
                                } else {
                                    &foreground
                                };
                                unsafe {
                                    surface.DrawTextLayout(
                                        origin,
                                        &run.layout,
                                        brush,
                                        D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                                    );
                                }
                            }
                            Item::Image(image) => unsafe {
                                if let Some(bitmap) = self.image(surface, &image.path) {
                                    let area = rect(group.x, y, image.width, image.height);
                                    surface.DrawBitmap(
                                        &bitmap,
                                        Some(&area),
                                        1.,
                                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                                        None,
                                    );
                                } else {
                                    surface.DrawTextLayout(
                                        point(group.x, y),
                                        &image.fallback.layout,
                                        &muted,
                                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                                    );
                                }
                            },
                        }
                    }
                    let max_x = group.maximum_horizontal();
                    if max_x > 0. {
                        let rail = self
                            .brush(&surface, if options.is_dark { 0x353b43 } else { 0xe5e9ef })?;
                        let thumb = self
                            .brush(&surface, if options.is_dark { 0x6e7786 } else { 0x9aa4b2 })?;
                        let thumb_width = (group.view_width * group.view_width
                            / group.content_width)
                            .max(18.)
                            .min(group.view_width);
                        let left = group.x + scroll_x / max_x * (group.view_width - thumb_width);
                        let row = y + group.height - 7.;
                        unsafe {
                            surface.FillRectangle(&rect(group.x, row, group.view_width, 5.), &rail);
                            surface.FillRectangle(&rect(left, row, thumb_width, 5.), &thumb);
                        }
                    }
                    Ok(())
                })();
                unsafe {
                    surface.PopAxisAlignedClip();
                }
                result?;
            }
            Ok(())
        })();
        operation.and(unsafe { surface.EndDraw(None, None) })
    }
}

fn bitmap(surface: &ID2D1RenderTarget, image: RgbaImage) -> Result<ID2D1Bitmap> {
    let size = D2D_SIZE_U {
        width: image.width(),
        height: image.height(),
    };
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        let alpha = pixel[3] as u32;
        for value in &mut pixel[..3] {
            *value = ((*value as u32 * alpha + 127) / 255) as u8;
        }
    }
    let props = D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.,
        dpiY: 96.,
    };
    unsafe { surface.CreateBitmap(size, Some(bgra.as_ptr().cast()), size.width * 4, &props) }
}

fn highlight(
    surface: &ID2D1RenderTarget,
    run: &TextRun,
    origin: D2D_POINT_2F,
    range: Range<usize>,
    brush: &ID2D1SolidColorBrush,
) {
    if run.fixed {
        return;
    }
    let lo = range.start.max(run.start);
    let hi = range.end.min(run.start + run.len);
    if hi <= lo {
        return;
    }
    let mut metrics = [DWRITE_HIT_TEST_METRICS::default(); 12];
    let mut count = 0;
    let start = (lo - run.start) as u32;
    let length = (hi - lo) as u32;
    unsafe {
        let ranges = if run
            .layout
            .HitTestTextRange(
                start,
                length,
                origin.x,
                origin.y,
                Some(&mut metrics),
                &mut count,
            )
            .is_ok()
        {
            &metrics[..count as usize]
        } else {
            if count == 0 || count > 100_000 {
                return;
            }
            return draw_highlights(surface, run, origin, start, length, count, brush);
        };
        for m in ranges {
            surface.FillRectangle(&rect(m.left, m.top, m.width, m.height), brush);
        }
    }
}

fn draw_highlights(
    surface: &ID2D1RenderTarget,
    run: &TextRun,
    origin: D2D_POINT_2F,
    start: u32,
    length: u32,
    mut count: u32,
    brush: &ID2D1SolidColorBrush,
) {
    let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
    unsafe {
        if run
            .layout
            .HitTestTextRange(
                start,
                length,
                origin.x,
                origin.y,
                Some(&mut metrics),
                &mut count,
            )
            .is_ok()
        {
            for m in metrics.iter().take(count as usize) {
                surface.FillRectangle(&rect(m.left, m.top, m.width, m.height), brush);
            }
        }
    }
}

struct TextRun {
    layout: IDWriteTextLayout,
    start: usize,
    len: usize,
    x: f32,
    y: f32,
    height: f32,
    fixed: bool,
    action: bool,
    effects: Vec<(DWRITE_TEXT_RANGE, u32)>,
    links: Vec<(Range<usize>, String)>,
    code_ranges: Vec<Range<usize>>,
}

enum Item {
    Text(TextRun),
    Image(Picture),
    Fill(f32, f32, f32, f32, u8),
}

struct Picture {
    path: Arc<PathBuf>,
    fallback: TextRun,
    width: f32,
    height: f32,
}

struct Group {
    block: usize,
    x: f32,
    y: f32,
    height: f32,
    content_width: f32,
    view_width: f32,
    items: Vec<Item>,
    code: Option<String>,
}

impl Group {
    fn maximum_horizontal(&self) -> f32 {
        (self.content_width - self.view_width).max(0.)
    }
}

#[derive(Default)]
pub struct Page {
    groups: Vec<Group>,
    contents: Vec<u16>,
    pub viewport_height: f32,
    pub scroll: f32,
    pub total_height: f32,
    pub selection: Option<(usize, usize)>,
    horizontal: HashMap<usize, f32>,
    matches: Vec<Range<usize>>,
    pub match_index: usize,
}

pub enum Hit {
    Text(usize),
    Link(String, usize),
    Code(String),
    Horizontal(usize, f32),
    None,
}

impl Page {
    pub fn build(
        factory: &IDWriteFactory,
        document: &Document,
        source: Option<&Path>,
        options: &Options,
        width: f32,
        height: f32,
    ) -> Result<Self> {
        let mut page = Self {
            viewport_height: height,
            ..Self::default()
        };
        let outer_inset = (width * 0.038).clamp(20., 44.);
        let available = (width - outer_inset * 2.).max(120.);
        let column = if options.auto_width {
            available
        } else {
            available.min(options.content_width)
        };
        let base_x = ((width - column) / 2.).max(8.);
        let mut y = 30.;
        let regular = if options.font_family == "mono" {
            w!("Consolas")
        } else if options.font_family == "serif" {
            w!("Georgia")
        } else {
            preferred_sans(factory)
        };
        let body_size = options.font_size;
        let mut formats = HashMap::new();
        let mut image_sources = HashMap::new();
        for (block_index, block) in document.blocks.iter().enumerate() {
            if block_index > 0 && block.indent == 0 && block.quote == 0 {
                if let Body::Heading(level, _) = block.body {
                    y += body_size * if level <= 2 { 0.92 } else { 0.53 };
                }
            }
            let indentation =
                (block.quote * 18 + block.indent * 22).min((column * 0.65) as usize) as f32;
            let mut group = Group {
                block: block_index,
                x: base_x + indentation,
                y,
                height: 0.,
                content_width: (column - indentation).max(120.),
                view_width: (column - indentation).max(120.),
                items: vec![],
                code: None,
            };
            let width = group.view_width;
            let mut builder = Builder {
                factory,
                contents: &mut page.contents,
                spacing: options.line_height,
                formats: &mut formats,
                chinese: options.font_family == "sans",
            };
            match &block.body {
                Body::Heading(level, spans) => {
                    let multiplier = match level {
                        1 => 1.72,
                        2 => 1.38,
                        3 => 1.16,
                        4 => 1.04,
                        _ => 1.0,
                    };
                    builder.spacing = 1.31;
                    let mut runs = spans.clone();
                    runs.iter_mut().for_each(|span| span.style.bold = true);
                    let run = builder.text(
                        &runs,
                        0.,
                        0.,
                        width,
                        body_size * multiplier,
                        regular,
                        true,
                        false,
                        TableAlignment::None,
                    )?;
                    group.height = run.height + 4.;
                    group.items.push(Item::Text(run));
                }
                Body::Text(spans) => {
                    let mut runs = spans.clone();
                    if !block.marker.is_empty() {
                        runs.insert(
                            0,
                            Span {
                                text: format!("{}  ", block.marker),
                                style: Style {
                                    marker: true,
                                    ..Default::default()
                                },
                            },
                        );
                    }
                    let run = builder.text(
                        &runs,
                        0.,
                        0.,
                        width,
                        body_size,
                        regular,
                        true,
                        false,
                        TableAlignment::None,
                    )?;
                    group.height = run.height;
                    group.items.push(Item::Text(run));
                }
                Body::Rule => {
                    group.items.push(Item::Fill(0., 5., width, 1., 1));
                    group.height = 12.;
                }
                Body::Code { language, text } => {
                    builder.spacing = 1.46;
                    let header = if language.is_empty() {
                        "代码"
                    } else {
                        language
                    };
                    let label = builder.text(
                        &[span(header)],
                        14.,
                        8.,
                        width - 80.,
                        11.5,
                        regular,
                        false,
                        true,
                        TableAlignment::None,
                    )?;
                    let mut button = builder.text(
                        &[span("复制代码")],
                        width - 63.,
                        8.,
                        60.,
                        11.5,
                        regular,
                        false,
                        true,
                        TableAlignment::None,
                    )?;
                    button.action = true;
                    let code_color = code_spans(text, language, options.is_dark);
                    let runs: Vec<_> = code_color.iter().map(|(run, _)| run.clone()).collect();
                    let mut code = builder.text(
                        &runs,
                        14.,
                        33.,
                        200_000.,
                        body_size * 0.86,
                        w!("Consolas"),
                        false,
                        false,
                        TableAlignment::None,
                    )?;
                    let mut start = 0;
                    for (span, ink) in code_color {
                        let length = span.text.encode_utf16().count() as u32;
                        if let Some(color) = ink {
                            code.effects.push((
                                DWRITE_TEXT_RANGE {
                                    startPosition: start,
                                    length,
                                },
                                color,
                            ));
                        }
                        start += length;
                    }
                    let mut metrics = DWRITE_TEXT_METRICS::default();
                    unsafe {
                        code.layout.GetMetrics(&mut metrics)?;
                    }
                    group.height = 33. + code.height + 12.;
                    group.content_width = width.max(metrics.widthIncludingTrailingWhitespace + 28.);
                    group
                        .items
                        .push(Item::Fill(0., 0., group.content_width, group.height, 0));
                    group.items.push(Item::Text(code));
                    group.items.push(Item::Text(label));
                    group.items.push(Item::Text(button));
                    group.code = Some(text.clone());
                }
                Body::Table {
                    rows,
                    align,
                    header,
                } => {
                    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
                    let minimum = (width / columns.max(1) as f32).clamp(75., 160.);
                    let widths: Vec<_> = (0..columns)
                        .map(|cell| {
                            let width = rows
                                .iter()
                                .filter_map(|row| row.get(cell))
                                .map(|spans| {
                                    plain(spans)
                                        .chars()
                                        .map(|c| {
                                            if c as u32 > 0x2fff {
                                                body_size
                                            } else {
                                                body_size * 0.51
                                            }
                                        })
                                        .sum::<f32>()
                                })
                                .fold(0., f32::max);
                            (width + 20.).clamp(minimum, 430.)
                        })
                        .collect();
                    let table_width = widths.iter().sum::<f32>();
                    let mut dy = 0.;
                    for (row_index, row) in rows.iter().enumerate() {
                        let mut x = 0.;
                        let mut line_height = body_size + 10.;
                        let mut cells = Vec::new();
                        for (index, cell) in row.iter().enumerate() {
                            let mut runs = cell.clone();
                            if *header && row_index == 0 {
                                runs.iter_mut().for_each(|run| run.style.bold = true);
                            }
                            let run = builder.text(
                                &runs,
                                x + 8.,
                                dy + 5.,
                                widths[index] - 16.,
                                body_size * 0.93,
                                regular,
                                true,
                                false,
                                align.get(index).copied().unwrap_or(TableAlignment::None),
                            )?;
                            if index + 1 < row.len() {
                                builder.contents.push('\t' as u16);
                            }
                            line_height = line_height.max(run.height + 10.);
                            cells.push(Item::Text(run));
                            x += widths[index];
                        }
                        builder.contents.push('\n' as u16);
                        if row_index % 2 == 0 {
                            group.items.push(Item::Fill(
                                0.,
                                dy,
                                table_width,
                                line_height,
                                if *header && row_index == 0 { 0 } else { 2 },
                            ));
                        }
                        group.items.push(Item::Fill(0., dy, table_width, 1., 1));
                        group.items.extend(cells);
                        dy += line_height;
                    }
                    let mut dx = 0.;
                    for cell in widths {
                        group.items.push(Item::Fill(dx, 0., 1., dy, 1));
                        dx += cell;
                    }
                    group.items.push(Item::Fill(0., dy, table_width, 1., 1));
                    group.items.push(Item::Fill(dx - 1., 0., 1., dy, 1));
                    group.content_width = width.max(table_width);
                    group.height = dy + 9.;
                }
                Body::Formula(display) => {
                    let run = builder.text(
                        &[span(display)],
                        0.,
                        0.,
                        width,
                        body_size,
                        w!("Consolas"),
                        true,
                        false,
                        TableAlignment::Center,
                    )?;
                    group.height = run.height;
                    group.items.push(Item::Text(run));
                }
                Body::Image { target, alt } => {
                    let metadata = source
                        .and_then(|document| image_path(document, target))
                        .and_then(|path| {
                            if image_sources.len() == IMAGE_SOURCE_LIMIT
                                && !image_sources.contains_key(&path)
                            {
                                return None;
                            }
                            image_sources
                                .entry(path)
                                .or_insert_with_key(|path| {
                                    let file = files::open_bounded(path, IMAGE_INPUT_LIMIT).ok()?;
                                    let size = ImageReader::new(BufReader::new(file))
                                        .with_guessed_format()
                                        .ok()?
                                        .into_dimensions()
                                        .ok()?;
                                    if size.0 == 0
                                        || size.1 == 0
                                        || size.0 > IMAGE_DIMENSION_LIMIT
                                        || size.1 > IMAGE_DIMENSION_LIMIT
                                    {
                                        return None;
                                    }
                                    Some((Arc::new(path.clone()), size))
                                })
                                .clone()
                        });
                    let fallback = builder.text(
                        &[span(format!("[图片] {alt}"))],
                        0.,
                        0.,
                        width,
                        body_size,
                        regular,
                        true,
                        false,
                        TableAlignment::None,
                    )?;
                    if let Some((path, (pixel_width, pixel_height))) = metadata {
                        let scale = (width / pixel_width as f32)
                            .min(600. / pixel_height as f32)
                            .min(1.);
                        let height = pixel_height as f32 * scale;
                        group.height = height.max(fallback.height);
                        group.items.push(Item::Image(Picture {
                            width: pixel_width as f32 * scale,
                            height,
                            path,
                            fallback,
                        }));
                    } else {
                        group.height = fallback.height;
                        group.items.push(Item::Text(fallback));
                    }
                }
            }
            if block.quote > 0 {
                // The quote bar sits inside the group's clip at every quote depth.
                group.items.push(Item::Fill(0., 0., 2., group.height, 3));
                for item in &mut group.items {
                    if let Item::Text(run) = item {
                        if !run.fixed {
                            run.x += 8.;
                        }
                    }
                }
            }
            page.contents.push('\n' as u16);
            y += group.height
                + if block.indent > 0 {
                    2.5
                } else {
                    body_size * 0.72
                };
            page.groups.push(group);
        }
        page.total_height = y + 32.;
        Ok(page)
    }

    pub fn maximum_scroll(&self) -> f32 {
        (self.total_height - self.viewport_height).max(0.)
    }
    pub fn set_scroll(&mut self, offset: f32) {
        self.scroll = offset.clamp(0., self.maximum_scroll());
    }
    pub fn progress(&self) -> f64 {
        if self.maximum_scroll() > 0. {
            (self.scroll / self.maximum_scroll()) as f64
        } else {
            0.
        }
    }
    pub fn jump(&mut self, block: usize) {
        if let Some(item) = self.groups.get(block) {
            self.set_scroll(item.y - 16.);
        }
    }

    pub fn hit(&self, x: f32, y: f32, links: bool) -> Hit {
        let doc_y = y + self.scroll;
        let pos = self
            .groups
            .partition_point(|group| group.y + group.height < doc_y);
        let Some(group) = self.groups.get(pos) else {
            return Hit::Text(self.contents.len());
        };
        let rel_y = (doc_y - group.y).max(0.);
        if links && x >= group.x && x <= group.x + group.view_width {
            if let Some(code) = &group.code {
                if rel_y <= 27. && x > group.x + group.view_width - 70. {
                    return Hit::Code(code.clone());
                }
            }
            if group.maximum_horizontal() > 0. && rel_y >= group.height - 10. {
                return Hit::Horizontal(
                    group.block,
                    (x - group.x) / group.view_width * group.maximum_horizontal(),
                );
            }
        }
        let shift = self
            .horizontal
            .get(&group.block)
            .copied()
            .unwrap_or_default()
            .min(group.maximum_horizontal());
        let rel_x = x.clamp(group.x, group.x + group.view_width) - group.x + shift;
        let candidate = group
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Text(run) if !run.fixed => Some(run),
                _ => None,
            })
            .min_by(|a, b| {
                let distance = |run: &TextRun| {
                    (rel_y - run.y).clamp(-1_000_000., 0.).abs()
                        + (rel_y - run.y - run.height).max(0.)
                        + ((rel_x - run.x).abs() * 0.0001)
                };
                distance(a).total_cmp(&distance(b))
            });
        let Some(run) = candidate else {
            return Hit::None;
        };
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        let mut trailing = BOOL(0);
        let mut inside = BOOL(0);
        unsafe {
            let _ = run.layout.HitTestPoint(
                rel_x - run.x,
                rel_y - run.y,
                &mut trailing,
                &mut inside,
                &mut metrics,
            );
        }
        let index = (metrics.textPosition
            + if trailing.as_bool() {
                metrics.length
            } else {
                0
            }) as usize;
        let absolute = run.start + index.min(run.len);
        if links && inside.as_bool() {
            if let Some((_, address)) = run
                .links
                .iter()
                .find(|(span, _)| span.contains(&(metrics.textPosition as usize)))
            {
                return Hit::Link(address.clone(), absolute);
            }
        }
        Hit::Text(absolute)
    }

    pub fn horizontal_scroll(&mut self, block: usize, target: f32) {
        if let Some(group) = self.groups.get(block) {
            self.horizontal
                .insert(block, target.clamp(0., group.maximum_horizontal()));
        }
    }

    pub fn shift_horizontal(&mut self, view_y: f32, difference: f32) {
        let y = self.scroll + view_y;
        if let Some(group) = self
            .groups
            .iter()
            .find(|group| group.y <= y && group.y + group.height >= y)
        {
            let current = self
                .horizontal
                .get(&group.block)
                .copied()
                .unwrap_or_default();
            self.horizontal_scroll(group.block, current + difference);
        }
    }

    pub fn selection_range(&self) -> Option<Range<usize>> {
        self.selection
            .and_then(|(a, b)| (a != b).then(|| a.min(b)..a.max(b)))
    }

    pub fn select_all(&mut self) {
        self.selection = Some((0, self.contents.len()));
    }
    pub fn selected(&self) -> Option<String> {
        self.selection_range()
            .and_then(|range| self.contents.get(range))
            .map(String::from_utf16_lossy)
    }

    pub fn search(&mut self, phrase: &str) -> usize {
        self.matches.clear();
        self.match_index = 0;
        if !phrase.is_empty() {
            let document = String::from_utf16_lossy(&self.contents);
            if let Ok(pattern) = regex::RegexBuilder::new(&regex::escape(phrase))
                .case_insensitive(true)
                .build()
            {
                let mut offset = 0;
                let mut previous = 0;
                for matched in pattern.find_iter(&document) {
                    offset += document[previous..matched.start()].encode_utf16().count();
                    let length = matched.as_str().encode_utf16().count();
                    self.matches.push(offset..offset + length);
                    offset += length;
                    previous = matched.end();
                }
            }
            self.locate_match();
        }
        self.matches.len()
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }
    fn matched_range(&self) -> Option<Range<usize>> {
        self.matches.get(self.match_index).cloned()
    }
    pub fn advance_match(&mut self, backward: bool) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.match_index = (self.match_index + if backward { count - 1 } else { 1 }) % count;
        self.locate_match();
    }
    fn locate_match(&mut self) {
        if let Some(range) = self.matched_range() {
            if let Some(group) = self.groups.iter().find(|group| group.items.iter().any(|item| matches!(item, Item::Text(run) if !run.fixed && range.start >= run.start && range.start < run.start + run.len))) {
                self.set_scroll(group.y - 20.);
            }
        }
    }
}

type FormatKey = (usize, u32, u32, bool, i32);

fn preferred_sans(factory: &IDWriteFactory) -> windows::core::PCWSTR {
    static VARIABLE: OnceLock<bool> = OnceLock::new();
    let available = VARIABLE.get_or_init(|| unsafe {
        let mut fonts = None;
        if factory.GetSystemFontCollection(&mut fonts, false).is_err() {
            return false;
        }
        let Some(fonts) = fonts else {
            return false;
        };
        let mut index = 0;
        let mut found = BOOL(0);
        fonts
            .FindFamilyName(w!("Segoe UI Variable Text"), &mut index, &mut found)
            .is_ok()
            && found.as_bool()
    });
    if *available {
        w!("Segoe UI Variable Text")
    } else {
        w!("Segoe UI")
    }
}

pub(crate) fn chinese_ranges(text: &[u16]) -> Vec<DWRITE_TEXT_RANGE> {
    let mut intervals = Vec::new();
    let mut offset = 0;
    let mut beginning = None;
    for character in char::decode_utf16(text.iter().copied()) {
        let value = character.unwrap_or(char::REPLACEMENT_CHARACTER);
        let is_cjk = matches!(value, '\u{2e80}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff01}'..='\u{ff60}' | '\u{20000}'..='\u{323af}');
        if is_cjk {
            beginning.get_or_insert(offset);
        } else if let Some(start) = beginning.take() {
            intervals.push(DWRITE_TEXT_RANGE {
                startPosition: start,
                length: offset - start,
            });
        }
        offset += value.len_utf16() as u32;
    }
    if let Some(start) = beginning {
        intervals.push(DWRITE_TEXT_RANGE {
            startPosition: start,
            length: offset - start,
        });
    }
    intervals
}

struct Builder<'a> {
    factory: &'a IDWriteFactory,
    contents: &'a mut Vec<u16>,
    spacing: f32,
    formats: &'a mut HashMap<FormatKey, IDWriteTextFormat>,
    chinese: bool,
}

impl Builder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        spans: &[Span],
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        family: windows::core::PCWSTR,
        wrap: bool,
        fixed: bool,
        alignment: TableAlignment,
    ) -> Result<TextRun> {
        let mut effects = Vec::new();
        let mut links = Vec::new();
        let mut code_ranges = Vec::new();
        let mut composed = Vec::new();
        let mut ranges = Vec::with_capacity(spans.len());
        for span in spans {
            let start = composed.len();
            composed.extend(span.text.encode_utf16());
            let end = composed.len();
            if end > start {
                ranges.push((
                    DWRITE_TEXT_RANGE {
                        startPosition: start as u32,
                        length: (end - start) as u32,
                    },
                    &span.style,
                ));
            }
        }
        let start = self.contents.len();
        if !fixed {
            self.contents.extend_from_slice(&composed);
        }
        unsafe {
            let alignment = match alignment {
                TableAlignment::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                TableAlignment::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
                _ => DWRITE_TEXT_ALIGNMENT_LEADING,
            };
            let height = size * self.spacing;
            let key = (
                family.0 as usize,
                size.to_bits(),
                self.spacing.to_bits(),
                wrap,
                alignment.0,
            );
            let format = match self.formats.entry(key) {
                std::collections::hash_map::Entry::Occupied(cached) => cached.into_mut(),
                std::collections::hash_map::Entry::Vacant(slot) => {
                    let format = self.factory.CreateTextFormat(
                        family,
                        None,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        size,
                        w!("zh-CN"),
                    )?;
                    format.SetWordWrapping(if wrap {
                        DWRITE_WORD_WRAPPING_WRAP
                    } else {
                        DWRITE_WORD_WRAPPING_NO_WRAP
                    })?;
                    format.SetTextAlignment(alignment)?;
                    format.SetLineSpacing(
                        DWRITE_LINE_SPACING_METHOD_UNIFORM,
                        height,
                        height * 0.76,
                    )?;
                    slot.insert(format)
                }
            };
            let layout =
                self.factory
                    .CreateTextLayout(&composed, &*format, width.max(20.), 100_000_000.)?;
            if self.chinese && family != w!("Consolas") {
                for range in chinese_ranges(&composed) {
                    layout.SetFontFamilyName(w!("Microsoft YaHei UI"), range)?;
                }
            }
            for (range, style) in ranges {
                if style.bold {
                    layout.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, range)?;
                }
                if style.italic {
                    layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?;
                }
                if style.strike {
                    layout.SetStrikethrough(true, range)?;
                }
                if style.code || style.formula {
                    layout.SetFontFamilyName(w!("Consolas"), range)?;
                }
                if style.marker {
                    layout.SetFontFamilyName(w!("Segoe UI Symbol"), range)?;
                    effects.push((range, 0x6a7788));
                }
                if style.code {
                    layout.SetFontSize(size * 0.90, range)?;
                    code_ranges.push(
                        start + range.startPosition as usize
                            ..start + (range.startPosition + range.length) as usize,
                    );
                }
                if let Some(link) = &style.link {
                    layout.SetUnderline(true, range)?;
                    effects.push((range, 0));
                    links.push((
                        range.startPosition as usize..(range.startPosition + range.length) as usize,
                        link.clone(),
                    ));
                }
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok(TextRun {
                layout,
                start,
                len: composed.len(),
                x,
                y,
                height: metrics.height.max(height),
                fixed,
                action: false,
                effects,
                links,
                code_ranges,
            })
        }
    }
}

fn span(text: impl Into<String>) -> Span {
    Span {
        text: text.into(),
        style: Style::default(),
    }
}

fn code_spans(text: &str, language: &str, dark: bool) -> Vec<(Span, Option<u32>)> {
    let plain = || vec![(span(text), None)];
    if text.len() > 256 * 1024 {
        return plain();
    }
    let language = language.to_ascii_lowercase();
    let Some(language) = Language::from_token(match language.as_str() {
        "shell" => "bash",
        "yml" => "yaml",
        other => other,
    }) else {
        return plain();
    };

    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight_raw(language, &text) else {
        return plain();
    };
    let mut colors = Vec::new();
    let mut result = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(kind)) => {
                colors.push(
                    HIGHLIGHT_NAMES
                        .get(kind.0)
                        .and_then(|name| code_color(name, dark)),
                );
            }
            Ok(HighlightEvent::HighlightEnd) => {
                colors.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(part) = text.get(start..end) else {
                    return plain();
                };
                result.push((span(part), colors.last().copied().flatten()));
            }
            Err(_) => return plain(),
        }
    }
    result
}

fn code_color(capture: &str, dark: bool) -> Option<u32> {
    let category = capture.split('.').next()?;
    let (light, dark_color) = match category {
        "comment" => (0x627077, 0x9ca7b6),
        "keyword" | "operator" => (0xa52354, 0xc694d1),
        "string" | "character" | "escape" => (0x19663b, 0xa6c891),
        "constant" | "number" | "boolean" => (0x0053a6, 0xe4ad78),
        "function" | "constructor" => (0x6a398e, 0x86b5e8),
        "type" | "tag" | "attribute" => (0x7a4a19, 0xe0c486),
        _ => return None,
    };
    Some(if dark { dark_color } else { light })
}

#[cfg(test)]
mod code_tests {
    use super::code_spans;

    #[test]
    fn code_highlighting_preserves_unicode_and_source() {
        let source = "fn main() { let text = \"Unicode: 中文\"; }\n";
        let spans = code_spans(source, "Rust", false);
        let restored: String = spans.iter().map(|(part, _)| part.text.as_str()).collect();
        assert_eq!(restored, source);
        assert!(spans.iter().any(|(_, color)| color.is_some()));
    }
}

fn image_path(source: &Path, target: &str) -> Option<PathBuf> {
    let path = percent_decode_str(target).decode_utf8().ok()?;
    if path.contains(':') || path.contains(['?', '#', '\0']) {
        return None;
    }
    let relative = Path::new(path.as_ref());
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(source.parent()?.join(relative))
}

fn image_reader(path: &Path) -> Option<ImageReader<Cursor<Vec<u8>>>> {
    let bytes = files::read_bounded(path, IMAGE_INPUT_LIMIT).ok()?;
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
}

fn load_image(path: &Path) -> Option<RgbaImage> {
    let mut reader = image_reader(path)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_image_height = Some(IMAGE_DIMENSION_LIMIT);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    if image.width() > IMAGE_BITMAP_DIMENSION || image.height() > IMAGE_BITMAP_DIMENSION {
        Some(
            image
                .thumbnail(IMAGE_BITMAP_DIMENSION, IMAGE_BITMAP_DIMENSION)
                .into_rgba8(),
        )
    } else {
        Some(image.into_rgba8())
    }
}
