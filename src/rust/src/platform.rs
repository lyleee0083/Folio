use crate::files::{self, wide};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Security::{Authorization::*, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    FlushFileBuffers, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const TRANSPORT_LIMIT: usize = 1_048_576;
pub const WM_LAUNCH: u32 = WM_APP + 1;
pub type LaunchQueue = Arc<Mutex<Vec<Vec<PathBuf>>>>;

pub fn command_line_paths() -> Vec<PathBuf> {
    std::env::args_os()
        .skip(1)
        .filter_map(|arg| std::path::absolute(arg).ok())
        .collect()
}

pub struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub enum Election {
    Primary(Instance, String),
    Secondary,
}

fn reveal_primary(process: u32) {
    let mut previous = HWND::default();
    unsafe {
        while let Ok(hwnd) = FindWindowExW(None, previous, super::WINDOW, None) {
            let mut owner = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
            if owner == process {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindowAsync(hwnd, SW_RESTORE);
                } else if !IsWindowVisible(hwnd).as_bool() {
                    let _ = ShowWindowAsync(hwnd, SW_SHOW);
                }
                let _ = SetForegroundWindow(hwnd);
                break;
            }
            previous = hwnd;
        }
    }
}

pub fn elect(profile: &Path, files: &[PathBuf]) -> Result<Election> {
    let mut session = 0;
    unsafe {
        let _ = ProcessIdToSessionId(std::process::id(), &mut session);
    }
    let identity = format!(
        "{}|{}|{}|{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default(),
        session,
        files::path_key(profile)
    );
    let hash = files::checksum(identity.as_bytes())
        .iter()
        .take(20)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mutex_name = wide(format!("Local\\Folio-Rust-{hash}"));
    let pipe = format!(r"\\.\pipe\Folio-Rust-{hash}");
    let handle = unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()))? };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let instance = Instance(handle);
    if !already_exists {
        return Ok(Election::Primary(instance, pipe));
    }
    let payload = serde_json::to_vec(files)
        .map_err(|error| windows::core::Error::new(E_FAIL, error.to_string()))?;
    if payload.len() > TRANSPORT_LIMIT {
        return Err(windows::core::Error::new(E_FAIL, "文件路径过多"));
    }
    let start = Instant::now();
    loop {
        match OpenOptions::new().read(true).write(true).open(&pipe) {
            Ok(mut channel) => {
                let mut process = 0;
                unsafe {
                    let _ =
                        GetNamedPipeServerProcessId(HANDLE(channel.as_raw_handle()), &mut process);
                    if process != 0 {
                        let _ = AllowSetForegroundWindow(process);
                    }
                }
                let exchange = (|| -> std::io::Result<u8> {
                    channel.write_all(&(payload.len() as u32).to_le_bytes())?;
                    channel.write_all(&payload)?;
                    let mut response = [0];
                    channel.read_exact(&mut response)?;
                    Ok(response[0])
                })();
                return match exchange {
                    Ok(1) => {
                        if process != 0 {
                            reveal_primary(process);
                        }
                        Ok(Election::Secondary)
                    }
                    _ => Err(windows::core::Error::new(
                        E_FAIL,
                        "已有 Folio 窗口无法接收文件",
                    )),
                };
            }
            Err(_) if start.elapsed() < Duration::from_secs(6) => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(error) => return Err(windows::core::Error::new(E_FAIL, error.to_string())),
        }
    }
}

pub fn listen(name: String, hwnd: HWND, queue: LaunchQueue) -> Result<()> {
    let window = hwnd.0 as usize;
    let pipe = wide(name);
    let mut descriptor = Default::default();
    unsafe {
        // Owner Rights grants access to the creating user's SID; remote clients are rejected independently.
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w!("D:P(A;;GA;;;OW)"),
            1,
            &mut descriptor,
            None,
        )?;
        let mut security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL(0),
        };
        let handle = CreateNamedPipeW(
            PCWSTR(pipe.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            8192,
            8192,
            0,
            Some(&mut security),
        );
        let _ = LocalFree(HLOCAL(descriptor.0));
        if handle == INVALID_HANDLE_VALUE {
            return Err(windows::core::Error::from_win32());
        }
        let raw = handle.0 as usize;
        std::thread::spawn(move || {
            let handle = HANDLE(raw as *mut c_void);
            let mut channel = File::from_raw_handle(handle.0);
            loop {
                if let Err(error) = ConnectNamedPipe(handle, None) {
                    if error.code() != ERROR_PIPE_CONNECTED.to_hresult() {
                        break;
                    }
                }
                let mut length = [0; 4];
                if channel.read_exact(&mut length).is_ok() {
                    let length = u32::from_le_bytes(length) as usize;
                    if length <= TRANSPORT_LIMIT {
                        let mut payload = vec![0; length];
                        if channel.read_exact(&mut payload).is_ok() {
                            if let Ok(paths) = serde_json::from_slice::<Vec<PathBuf>>(&payload) {
                                if let Ok(mut pending) = queue.lock() {
                                    pending.push(paths);
                                    let _ = PostMessageW(
                                        HWND(window as *mut c_void),
                                        WM_LAUNCH,
                                        WPARAM(0),
                                        LPARAM(0),
                                    );
                                    let _ = channel.write_all(&[1]);
                                    // The client closes after reading the acknowledgement. Disconnect
                                    // only after the byte left the server buffer.
                                    let _ = FlushFileBuffers(handle);
                                }
                            }
                        }
                    }
                }
                let _ = DisconnectNamedPipe(handle);
            }
        });
    }
    Ok(())
}

pub fn choose_files(owner: HWND, folders: bool) -> Result<Vec<PathBuf>> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
        options |= if folders {
            FOS_PICKFOLDERS
        } else {
            FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST
        };
        dialog.SetOptions(options)?;
        if !folders {
            dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: w!("Markdown 文档"),
                pszSpec: w!("*.md;*.markdown;*.mdown;*.txt"),
            }])?;
        }
        if let Err(error) = dialog.Show(owner) {
            if error.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for i in 0..items.GetCount()? {
            let name = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0.cast()));
            if let Ok(path) = path {
                paths.push(PathBuf::from(path));
            }
        }
        Ok(paths)
    }
}

pub fn clipboard(owner: HWND, text: &str) -> Result<()> {
    unsafe {
        OpenClipboard(owner)?;
        let operation = (|| {
            EmptyClipboard()?;
            let contents = wide(text);
            let bytes = contents.len() * size_of::<u16>();
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let data = GlobalLock(handle);
            if data.is_null() {
                let _ = GlobalFree(handle);
                return Err(windows::core::Error::from_win32());
            }
            std::ptr::copy_nonoverlapping(contents.as_ptr().cast::<u8>(), data.cast::<u8>(), bytes);
            let _ = GlobalUnlock(handle);
            if let Err(error) = SetClipboardData(13, HANDLE(handle.0)) {
                let _ = GlobalFree(handle);
                return Err(error);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        operation
    }
}

pub fn drag_paths(drop: HDROP) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    unsafe {
        let count = DragQueryFileW(drop, u32::MAX, None).min(2000);
        for index in 0..count {
            let len = DragQueryFileW(drop, index, None);
            let mut path = vec![0; len as usize + 1];
            let copied = DragQueryFileW(drop, index, Some(&mut path));
            if copied > 0 {
                paths.push(PathBuf::from(String::from_utf16_lossy(
                    &path[..copied as usize],
                )));
            }
        }
        DragFinish(drop);
    }
    paths
}

pub fn error_dialog(hwnd: HWND, error: impl std::fmt::Display) {
    unsafe {
        MessageBoxW(
            hwnd,
            PCWSTR(wide(error.to_string()).as_ptr()),
            w!("Folio"),
            MB_ICONERROR | MB_OK,
        );
    }
}

pub fn print_page(
    owner: HWND,
    document: &crate::document::Document,
    source: &Path,
    options: &files::Options,
) -> Result<()> {
    use crate::preview::{Page, Renderer};
    unsafe {
        let mut dialog = PRINTDLGW {
            lStructSize: size_of::<PRINTDLGW>() as u32,
            hwndOwner: owner,
            Flags: PD_RETURNDC | PD_NOSELECTION | PD_NOPAGENUMS,
            nCopies: 1,
            ..Default::default()
        };
        let chosen = PrintDlgW(&mut dialog).as_bool();
        for handle in [dialog.hDevMode, dialog.hDevNames] {
            if !handle.0.is_null() {
                let _ = GlobalFree(handle);
            }
        }
        if !chosen {
            return Ok(());
        }
        let dc = dialog.hDC;
        let print = (|| {
            let xdpi = GetDeviceCaps(dc, LOGPIXELSX).max(96) as f32;
            let ydpi = GetDeviceCaps(dc, LOGPIXELSY).max(96) as f32;
            let margin_x = (xdpi * 0.28) as i32;
            let margin_y = (ydpi * 0.32) as i32;
            let right = GetDeviceCaps(dc, HORZRES) - margin_x;
            let bottom = GetDeviceCaps(dc, VERTRES) - margin_y;
            let area = RECT {
                left: margin_x,
                top: margin_y,
                right,
                bottom,
            };
            let width = (right - margin_x) as f32 * 96. / xdpi;
            let height = (bottom - margin_y) as f32 * 96. / ydpi;
            if width < 100. || height < 100. {
                return Err(windows::core::Error::new(E_FAIL, "打印区域太小"));
            }
            let mut options = options.clone();
            options.is_dark = false;
            options.auto_width = false;
            options.content_width = (options.font_size * 47.5).clamp(660., 920.);
            let mut renderer = Renderer::new()?;
            let mut page = Page::build(
                &renderer.write,
                document,
                Some(source),
                &options,
                width,
                height,
            )?;
            let target = renderer.printer_target(xdpi, ydpi)?;
            let name = wide(source.file_name().unwrap_or_default());
            let info = DOCINFOW {
                cbSize: size_of::<DOCINFOW>() as i32,
                lpszDocName: PCWSTR(name.as_ptr()),
                ..Default::default()
            };
            if StartDocW(dc, &info) <= 0 {
                return Err(windows::core::Error::from_win32());
            }
            let output = (|| {
                let mut offset = 0.;
                while offset < page.total_height {
                    if StartPage(dc) <= 0 {
                        return Err(windows::core::Error::from_win32());
                    }
                    target.BindDC(dc, &area)?;
                    page.scroll = offset;
                    renderer.draw_target(&target, &page, &options)?;
                    if EndPage(dc) <= 0 {
                        return Err(windows::core::Error::from_win32());
                    }
                    offset += height;
                }
                Ok(())
            })();
            if output.is_err() {
                AbortDoc(dc);
            } else if EndDoc(dc) <= 0 {
                return Err(windows::core::Error::from_win32());
            }
            output
        })();
        let _ = DeleteDC(dc);
        print
    }
}
