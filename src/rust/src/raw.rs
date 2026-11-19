use std::ffi::c_void;
use std::sync::OnceLock;
use windows::core::{IUnknown, Interface};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::TA_BASELINE;
use windows::Win32::UI::Controls::RichEdit::*;
use windows::Win32::UI::Controls::{
    EM_EMPTYUNDOBUFFER, EM_GETFIRSTVISIBLELINE, EM_GETLINECOUNT, EM_GETMODIFY, EM_LINEINDEX,
    EM_LINESCROLL, EM_SCROLL, EM_SCROLLCARET, EM_SETMODIFY, EM_SETSEL,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_VSCROLL};

pub struct LineIndex {
    starts: Vec<i32>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut starts = vec![0];
        let mut offset = 0;
        for character in source.chars() {
            offset += character.len_utf16() as i32;
            if character == '\n' {
                starts.push(offset);
            }
        }
        Self { starts }
    }

    pub fn cursor(&self, position: i32) -> (usize, i32) {
        let position = position.max(0);
        let line = self.starts.partition_point(|start| *start <= position);
        (line, position - self.starts[line - 1] + 1)
    }

    pub unsafe fn visible_rows(&self, editor: HWND, limit: usize) -> Vec<(i32, usize)> {
        let Some(range) = document(editor).and_then(|doc| doc.Range(0, 0).ok()) else {
            return Vec::new();
        };
        let first = super::send(editor, EM_GETFIRSTVISIBLELINE, 0, 0).max(0) as usize;
        let end = super::send(editor, EM_GETLINECOUNT, 0, 0).max(0) as usize;
        let mut rows = Vec::new();
        for visual in first..end.min(first + limit) {
            let position = super::send(editor, EM_LINEINDEX, visual, 0) as i32;
            if let Ok(line) = self.starts.binary_search(&position) {
                if let Some(y) = baseline(&range, position) {
                    rows.push((y, line + 1));
                }
            }
        }
        rows
    }
}

pub(super) unsafe extern "system" fn editor_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    gutter: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(editor_proc), id);
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if matches!(
        message,
        WM_PAINT
            | WM_VSCROLL
            | WM_MOUSEWHEEL
            | EM_SCROLL
            | EM_LINESCROLL
            | EM_SCROLLCARET
            | EM_SETSCROLLPOS
    ) {
        super::invalidate(HWND(gutter as *mut c_void));
    }
    result
}

fn tokens() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(
        r"(?m)(?<heading>^ {0,3}#{1,6})(?:[ \t]|$)|(?<code>^ {0,3}(?:`{3,}|~{3,}).*$|`[^`\r\n]+`)|^[\t ]*\|(?:[ \t]*:?-{3,}:?[ \t]*\|)+[\t ]*$|^[\t ]*(?:[-+*>]|\d+[.)])(?:[\t ]+\[[ xX]\])?[\t ]+"
    ).expect("valid Raw highlighting pattern"))
}

unsafe fn document(control: HWND) -> Option<ITextDocument> {
    let mut pointer: *mut c_void = std::ptr::null_mut();
    if super::send(
        control,
        EM_GETOLEINTERFACE,
        0,
        &mut pointer as *mut _ as isize,
    ) == 0
        || pointer.is_null()
    {
        return None;
    }
    let rich = IUnknown::from_raw(pointer);
    rich.cast().ok()
}

unsafe fn baseline(range: &ITextRange, position: i32) -> Option<i32> {
    range.SetRange(position, position).ok()?;
    let mut x = 0;
    let mut y = i32::MIN;
    range
        .GetPoint(
            tomConstants(
                tomStart.0 | tomClientCoord.0 | tomAllowOffClient.0 | TA_BASELINE.0 as i32,
            ),
            &mut x,
            &mut y,
        )
        .ok()?;
    (y != i32::MIN).then_some(y)
}

pub(super) unsafe fn space_lines(control: HWND) {
    let format = PARAFORMAT2 {
        Base: PARAFORMAT {
            cbSize: size_of::<PARAFORMAT2>() as u32,
            dwMask: PFM_LINESPACING,
            ..Default::default()
        },
        dyLineSpacing: 315, // 21 DIPs in twips.
        bLineSpacingRule: tomLineSpaceAtLeast.0 as u8,
        ..Default::default()
    };
    super::send(control, EM_SETSEL, 0, -1);
    super::send(control, EM_SETPARAFORMAT, 0, &format as *const _ as isize);
    super::send(control, EM_SETSEL, 0, 0);
    super::send(control, EM_EMPTYUNDOBUFFER, 0, 0);
}

unsafe fn appearance(control: HWND, ink: u32, all: bool) {
    let format = CHARFORMATW {
        cbSize: size_of::<CHARFORMATW>() as u32,
        dwMask: CFM_COLOR | CFM_BOLD,
        crTextColor: COLORREF(ink),
        ..Default::default()
    };
    super::send(
        control,
        EM_SETCHARFORMAT,
        if all { SCF_ALL } else { SCF_SELECTION } as usize,
        &format as *const _ as isize,
    );
}

fn bgr(rgb: u32) -> u32 {
    ((rgb & 0xff) << 16) | (rgb & 0x00ff00) | ((rgb >> 16) & 0xff)
}

pub unsafe fn highlight(control: HWND, dark: bool) {
    let Some(doc) = document(control) else {
        return;
    };
    let Ok(_) = doc.Undo(tomSuspend.0) else {
        return;
    };
    let _ = doc.Freeze();
    let dirty = super::send(control, EM_GETMODIFY, 0, 0);
    let mut selection = CHARRANGE::default();
    let mut scroll = POINT::default();
    super::send(control, EM_EXGETSEL, 0, &mut selection as *mut _ as isize);
    super::send(control, EM_GETSCROLLPOS, 0, &mut scroll as *mut _ as isize);
    let text = crate::files::normalize_source(&super::get_text(control));
    appearance(control, bgr(if dark { 0xeceef2 } else { 0x1e2024 }), true);
    if text.len() <= 500_000 {
        let mut previous = 0;
        let mut offset = 0;
        for groups in tokens().captures_iter(&text) {
            let Some(token) = groups.name("heading").or_else(|| groups.get(0)) else {
                continue;
            };
            offset += text[previous..token.start()].encode_utf16().count();
            let length = token.as_str().encode_utf16().count();
            let range = CHARRANGE {
                cpMin: offset as i32,
                cpMax: (offset + length) as i32,
            };
            let color = if groups.name("code").is_some() {
                if dark {
                    0xa6c7de
                } else {
                    0x3b667e
                }
            } else {
                if dark {
                    0x9aa5b3
                } else {
                    0x677180
                }
            };
            super::send(control, EM_EXSETSEL, 0, &range as *const _ as isize);
            appearance(control, bgr(color), false);
            offset += length;
            previous = token.end();
        }
    }
    super::send(control, EM_EXSETSEL, 0, &selection as *const _ as isize);
    super::send(control, EM_SETSCROLLPOS, 0, &scroll as *const _ as isize);
    super::send(control, EM_SETMODIFY, dirty as usize, 0);
    let _ = doc.Unfreeze();
    let _ = doc.Undo(tomResume.0);
    super::invalidate(control);
}
