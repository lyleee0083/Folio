// Lucide 1.74.0, ISC and Feather MIT. See docs/third-party-licenses/Lucide-LICENSE.txt.
use crate::preview::chinese_ranges;
use svgtypes::{SimplePathSegment, SimplifyingPathParser};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{BOOL, RECT};
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED};

pub struct Painter {
    factory: ID2D1Factory,
    target: ID2D1DCRenderTarget,
    write: IDWriteFactory,
    label_family: PCWSTR,
}

#[derive(Default)]
pub struct Shell {
    pub sidebar: i32,
    pub outline: i32,
    pub toolbar_bottom: i32,
    pub footer_top: i32,
    pub filter: Option<RECT>,
}

pub struct Tone {
    pub canvas: u32,
    pub rail: u32,
    pub line: u32,
    pub text: u32,
    pub muted: u32,
    pub selected: u32,
}

impl Tone {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                canvas: 0x1e2124,
                rail: 0x191c20,
                line: 0x343943,
                text: 0xeceef2,
                muted: 0xa0a7b4,
                selected: 0x2a3038,
            }
        } else {
            Self {
                canvas: 0xffffff,
                rail: 0xf7f9fc,
                line: 0xe4e9f0,
                text: 0x20242a,
                muted: 0x656f7e,
                selected: 0xe9edf2,
            }
        }
    }
}

impl Painter {
    pub fn new() -> Result<Self> {
        let factory: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.,
            dpiY: 96.,
            ..Default::default()
        };
        let target = unsafe { factory.CreateDCRenderTarget(&props)? };
        let write: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let label_family = unsafe {
            let mut fonts = None;
            let family = w!("Microsoft YaHei UI");
            let _ = write.GetSystemFontCollection(&mut fonts, false);
            let mut index = 0;
            let mut found = BOOL::default();
            if let Some(fonts) = fonts {
                let _ = fonts.FindFamilyName(family, &mut index, &mut found);
            }
            if found.as_bool() {
                family
            } else {
                w!("Segoe UI")
            }
        };
        Ok(Self {
            factory,
            target,
            write,
            label_family,
        })
    }

    pub fn shell(
        &self,
        dc: HDC,
        bounds: &RECT,
        shell: &Shell,
        tone: &Tone,
        scale: f32,
    ) -> Result<()> {
        let width = bounds.right as f32;
        let top = shell.toolbar_bottom as f32;
        let bottom = shell.footer_top as f32;
        unsafe {
            self.target.BindDC(dc, bounds)?;
            let rail = self.target.CreateSolidColorBrush(&color(tone.rail), None)?;
            let line = self.target.CreateSolidColorBrush(&color(tone.line), None)?;
            let canvas = self
                .target
                .CreateSolidColorBrush(&color(tone.canvas), None)?;
            self.target.BeginDraw();
            self.target.Clear(Some(&color(tone.canvas)));
            if shell.sidebar > 0 {
                let side = shell.sidebar as f32;
                self.target
                    .FillRectangle(&area(0., top, side, (bottom - top).max(0.)), &rail);
                self.target
                    .FillRectangle(&area(side - 1., top, 1., (bottom - top).max(0.)), &line);
            }
            if shell.outline > 0 {
                let start = width - shell.outline as f32;
                self.target.FillRectangle(
                    &area(start, top, shell.outline as f32, (bottom - top).max(0.)),
                    &rail,
                );
                self.target
                    .FillRectangle(&area(start, top, 1., (bottom - top).max(0.)), &line);
            }
            self.target
                .FillRectangle(&area(0., top - 1., width, 1.), &line);
            self.target
                .FillRectangle(&area(0., bottom, width, 1.), &line);
            if let Some(filter) = shell.filter {
                let field = D2D1_ROUNDED_RECT {
                    rect: area(
                        filter.left as f32,
                        filter.top as f32,
                        (filter.right - filter.left) as f32,
                        (filter.bottom - filter.top) as f32,
                    ),
                    radiusX: 6. * scale,
                    radiusY: 6. * scale,
                };
                self.target.FillRoundedRectangle(&field, &canvas);
                self.target.DrawRoundedRectangle(&field, &line, 1., None);
            }
            self.target.EndDraw(None, None)
        }
    }

    pub fn list_row(
        &self,
        item: &DRAWITEMSTRUCT,
        label: &str,
        tone: &Tone,
        dpi: u32,
    ) -> Result<()> {
        let width = (item.rcItem.right - item.rcItem.left) as f32;
        let height = (item.rcItem.bottom - item.rcItem.top) as f32;
        let scale = dpi as f32 / 96.;
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
        unsafe {
            self.target.BindDC(item.hDC, &item.rcItem)?;
            let text = self.target.CreateSolidColorBrush(&color(tone.text), None)?;
            let shade = self
                .target
                .CreateSolidColorBrush(&color(tone.selected), None)?;
            let focus = self
                .target
                .CreateSolidColorBrush(&color(tone.muted), None)?;
            self.target.BeginDraw();
            self.target.Clear(Some(&color(tone.rail)));
            let area = area(1., 1., width - 2., height - 2.);
            let highlight = D2D1_ROUNDED_RECT {
                rect: area,
                radiusX: 6. * scale,
                radiusY: 6. * scale,
            };
            if selected {
                self.target.FillRoundedRectangle(&highlight, &shade);
            }
            if focused {
                self.target
                    .DrawRoundedRectangle(&highlight, &focus, 1., None);
            }
            let result = self.label(
                label,
                &text,
                (11. * scale, 0., (width - 21. * scale).max(1.), height),
                13. * scale,
            );
            result.and(self.target.EndDraw(None, None))
        }
    }

    pub fn action(
        &self,
        item: &DRAWITEMSTRUCT,
        icon: &str,
        label: &str,
        tone: &Tone,
        dpi: u32,
    ) -> Result<()> {
        let width = (item.rcItem.right - item.rcItem.left) as f32;
        let height = (item.rcItem.bottom - item.rcItem.top) as f32;
        let scale = dpi as f32 / 96.;
        let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
        unsafe {
            self.target.BindDC(item.hDC, &item.rcItem)?;
            let ink = self.target.CreateSolidColorBrush(&color(tone.text), None)?;
            let line = self.target.CreateSolidColorBrush(&color(tone.line), None)?;
            let background = self.target.CreateSolidColorBrush(
                &color(if selected { tone.selected } else { tone.canvas }),
                None,
            )?;
            self.target.BeginDraw();
            self.target.Clear(Some(&color(tone.rail)));
            let button = D2D1_ROUNDED_RECT {
                rect: area(1., 1., width - 2., height - 2.),
                radiusX: 6. * scale,
                radiusY: 6. * scale,
            };
            self.target.FillRoundedRectangle(&button, &background);
            self.target.DrawRoundedRectangle(&button, &line, 1., None);
            let x = ((width - 51. * scale) * 0.5).max(6. * scale);
            let icon_result = self.geometry(
                icon,
                &ink,
                x,
                (height - 14. * scale) * 0.5,
                14. * scale / 24.,
            );
            let label_result = icon_result.and_then(|_| {
                self.label(
                    label,
                    &ink,
                    (x + 20. * scale, 0., 36. * scale, height),
                    13. * scale,
                )
            });
            label_result.and(self.target.EndDraw(None, None))
        }
    }

    unsafe fn label(
        &self,
        value: &str,
        brush: &ID2D1SolidColorBrush,
        bounds: (f32, f32, f32, f32),
        size: f32,
    ) -> Result<()> {
        let format = self.write.CreateTextFormat(
            self.label_family,
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("zh-CN"),
        )?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let trim = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        let ellipsis = self.write.CreateEllipsisTrimmingSign(&format)?;
        format.SetTrimming(&trim, &ellipsis)?;
        let encoded = value.encode_utf16().collect::<Vec<_>>();
        let layout = self
            .write
            .CreateTextLayout(&encoded, &format, bounds.2, bounds.3)?;
        // Chinese glyphs fill more of the same nominal font size than Latin lowercase.
        for range in chinese_ranges(&encoded) {
            layout.SetFontSize(size * (12. / 13.), range)?;
        }
        self.target.DrawTextLayout(
            D2D_POINT_2F {
                x: bounds.0,
                y: bounds.1,
            },
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
        Ok(())
    }

    pub fn draw(
        &self,
        name: &str,
        item: &DRAWITEMSTRUCT,
        dark: bool,
        state: bool,
        dpi: u32,
    ) -> Result<()> {
        let width = (item.rcItem.right - item.rcItem.left) as f32;
        let height = (item.rcItem.bottom - item.rcItem.top) as f32;
        let scale = dpi as f32 / 96.;
        let on = item.itemState.0 & ODS_SELECTED.0 != 0 || state;
        let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
        let base = color(if dark { 0x1e2124 } else { 0xffffff });
        let active = color(if dark { 0x363e49 } else { 0xe7edf5 });
        let stroke = color(match (dark, disabled) {
            (false, false) => 0x202020,
            (false, true) => 0xa0a0a0,
            (true, false) => 0xeeeeee,
            (true, true) => 0x808080,
        });
        unsafe {
            self.target.BindDC(item.hDC, &item.rcItem)?;
            let ink = self.target.CreateSolidColorBrush(&stroke, None)?;
            let background = self.target.CreateSolidColorBrush(&active, None)?;
            self.target.BeginDraw();
            self.target.Clear(Some(&base));
            let rectangle = D2D1_ROUNDED_RECT {
                rect: area(1., 1., width - 2., height - 2.),
                radiusX: 5. * scale,
                radiusY: 5. * scale,
            };
            if on {
                self.target.FillRoundedRectangle(&rectangle, &background);
            }
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                self.target
                    .DrawRoundedRectangle(&rectangle, &ink, scale, None);
            }
            let result = self.geometry(
                name,
                &ink,
                (width - 16. * scale) * 0.5,
                (height - 16. * scale) * 0.5,
                16. * scale / 24.,
            );
            result.and(self.target.EndDraw(None, None))
        }
    }

    unsafe fn geometry(
        &self,
        label: &str,
        ink: &ID2D1SolidColorBrush,
        left: f32,
        top: f32,
        scale: f32,
    ) -> Result<()> {
        let style = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.,
        };
        let style = self.factory.CreateStrokeStyle(&style, None)?;
        let transform = |x: f64, y: f64| D2D_POINT_2F {
            x: left + x as f32 * scale,
            y: top + y as f32 * scale,
        };
        for shape in shapes(label) {
            match shape {
                Shape::Circle(x, y, radius) => {
                    let e = D2D1_ELLIPSE {
                        point: transform(*x as f64, *y as f64),
                        radiusX: radius * scale,
                        radiusY: radius * scale,
                    };
                    self.target.DrawEllipse(&e, ink, scale * 2., &style);
                }
                Shape::Rounded(x, y, width, height, radius) => {
                    let r = D2D1_ROUNDED_RECT {
                        rect: area(
                            left + x * scale,
                            top + y * scale,
                            width * scale,
                            height * scale,
                        ),
                        radiusX: radius * scale,
                        radiusY: radius * scale,
                    };
                    self.target
                        .DrawRoundedRectangle(&r, ink, scale * 2., &style);
                }
                Shape::Path(source) => {
                    let geometry = self.factory.CreatePathGeometry()?;
                    let sink = geometry.Open()?;
                    let mut open = false;
                    for segment in SimplifyingPathParser::from(*source).flatten() {
                        match segment {
                            SimplePathSegment::MoveTo { x, y } => {
                                if open {
                                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                                }
                                sink.BeginFigure(transform(x, y), D2D1_FIGURE_BEGIN_HOLLOW);
                                open = true;
                            }
                            SimplePathSegment::LineTo { x, y } => sink.AddLine(transform(x, y)),
                            SimplePathSegment::CurveTo {
                                x1,
                                y1,
                                x2,
                                y2,
                                x,
                                y,
                            } => {
                                sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                                    point1: transform(x1, y1),
                                    point2: transform(x2, y2),
                                    point3: transform(x, y),
                                });
                            }
                            SimplePathSegment::Quadratic { x1, y1, x, y } => {
                                sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT {
                                    point1: transform(x1, y1),
                                    point2: transform(x, y),
                                });
                            }
                            SimplePathSegment::ClosePath => {
                                if open {
                                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                                    open = false;
                                }
                            }
                        }
                    }
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    sink.Close()?;
                    self.target.DrawGeometry(&geometry, ink, scale * 2., &style);
                }
            }
        }
        Ok(())
    }
}

fn area(x: f32, y: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    }
}

fn color(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: (hex >> 16 & 255) as f32 / 255.,
        g: (hex >> 8 & 255) as f32 / 255.,
        b: (hex & 255) as f32 / 255.,
        a: 1.,
    }
}

enum Shape {
    Path(&'static str),
    Circle(f32, f32, f32),
    Rounded(f32, f32, f32, f32, f32),
}

fn shapes(icon: &str) -> &'static [Shape] {
    match icon {
        "panel-left" => &[Shape::Rounded(3., 3., 18., 18., 2.), Shape::Path("M9 3v18")],
        "pencil" => &[Shape::Path("M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"), Shape::Path("m15 5 4 4")],
        "eye" => &[Shape::Path("M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"), Shape::Circle(12., 12., 3.)],
        "save" => &[Shape::Path("M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"), Shape::Path("M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7"), Shape::Path("M7 3v4a1 1 0 0 0 1 1h7")],
        "search" => &[Shape::Path("m21 21-4.34-4.34"), Shape::Circle(11., 11., 8.)],
        "list" => &[Shape::Path("M3 5h.01"), Shape::Path("M3 12h.01"), Shape::Path("M3 19h.01"), Shape::Path("M8 5h13"), Shape::Path("M8 12h13"), Shape::Path("M8 19h13")],
        "moon" => &[Shape::Path("M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401")],
        "settings" => &[Shape::Path("M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915"), Shape::Circle(12., 12., 3.)],
        "ellipsis" => &[Shape::Circle(12., 12., 1.), Shape::Circle(19., 12., 1.), Shape::Circle(5., 12., 1.)],
        "folder-open" => &[Shape::Path("m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2")],
        "folder" => &[Shape::Path("M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z")],
        "file-text" => &[Shape::Path("M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"), Shape::Path("M14 2v5a1 1 0 0 0 1 1h5"), Shape::Path("M10 9H8"), Shape::Path("M16 13H8"), Shape::Path("M16 17H8")],
        "chevron-up" => &[Shape::Path("m18 15-6-6-6 6")],
        "chevron-down" => &[Shape::Path("m6 9 6 6 6-6")],
        "x" => &[Shape::Path("M18 6 6 18"), Shape::Path("m6 6 12 12")],
        "copy" => &[Shape::Rounded(8., 8., 14., 14., 2.), Shape::Path("M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2")],
        _ => &[],
    }
}
