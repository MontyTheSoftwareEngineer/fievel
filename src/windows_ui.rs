use crate::{
    engine::SpeedMode,
    font::{Canvas, Color},
    input::KeyCode,
};
use std::{
    io,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    },
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
            MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
            GetSystemMetrics, PeekMessageW, PostQuitMessage, RegisterClassW, SetWindowPos,
            ShowWindow, TranslateMessage, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW,
            HWND_TOPMOST, MSG, PM_REMOVE, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN,
            SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE,
            SWP_NOOWNERZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_DESTROY,
            WM_ERASEBKGND, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
};

const CURSOR_PULSE_DURATION: Duration = Duration::from_millis(750);
pub const INPUT_MARKER: usize = 0x6669_6576_656c_6d63u64 as usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayHint {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub label: String,
    pub prefix_length: usize,
    pub border: Option<Color>,
    pub fill: Option<Color>,
    pub debug_dot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintColors {
    pub border: Color,
    pub fill: Color,
    pub readability: Color,
    pub text: Color,
    pub highlight: Color,
}

pub struct Overlay {
    hwnd: HWND,
    screen_dc: HDC,
    memory_dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    pixels: *mut u8,
    origin: (i32, i32),
    primary_size: (i32, i32),
    size: (u32, u32),
    indicator: Option<String>,
    hints: Vec<OverlayHint>,
    hint_colors: HintColors,
    hints_hidden: bool,
    pulse: Option<((i32, i32), Instant)>,
    temporarily_hidden: bool,
    visible: bool,
}

impl Overlay {
    pub fn new() -> io::Result<Self> {
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let origin = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        let primary_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let primary_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        if width <= 0 || height <= 0 || primary_width <= 0 || primary_height <= 0 {
            return Err(io::Error::other(
                "Windows reported an invalid virtual screen size",
            ));
        }
        let byte_count = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| io::Error::other("virtual screen is too large"))?;
        let screen_dc = unsafe { GetDC(null_mut()) };
        if screen_dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
        if memory_dc.is_null() {
            let error = io::Error::last_os_error();
            unsafe { ReleaseDC(null_mut(), screen_dc) };
            return Err(error);
        }

        let mut info: BITMAPINFO = unsafe { zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        };
        let mut bits = null_mut();
        let bitmap =
            unsafe { CreateDIBSection(screen_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
        if bitmap.is_null() || bits.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                if !bitmap.is_null() {
                    DeleteObject(bitmap);
                }
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
            }
            return Err(error);
        }
        unsafe { std::ptr::write_bytes(bits.cast::<u8>(), 0, byte_count) };
        let previous_bitmap = unsafe { SelectObject(memory_dc, bitmap) };
        if previous_bitmap.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                DeleteObject(bitmap);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
            }
            return Err(error);
        }

        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                SelectObject(memory_dc, previous_bitmap);
                DeleteObject(bitmap);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
            }
            return Err(error);
        }
        let class_name = wide("FievelOverlayWindow");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { zeroed() }
        };
        let registered = unsafe { RegisterClassW(&class) };
        if registered == 0 {
            let error_code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if error_code != windows_sys::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS {
                let error = io::Error::from_raw_os_error(error_code as i32);
                unsafe {
                    SelectObject(memory_dc, previous_bitmap);
                    DeleteObject(bitmap);
                    DeleteDC(memory_dc);
                    ReleaseDC(null_mut(), screen_dc);
                }
                return Err(error);
            }
        }
        let title = wide("Fievel");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                origin.0,
                origin.1,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if hwnd.is_null() {
            let error = io::Error::last_os_error();
            unsafe {
                SelectObject(memory_dc, previous_bitmap);
                DeleteObject(bitmap);
                DeleteDC(memory_dc);
                ReleaseDC(null_mut(), screen_dc);
            }
            return Err(error);
        }
        let overlay = Self {
            hwnd,
            screen_dc,
            memory_dc,
            bitmap,
            previous_bitmap,
            pixels: bits.cast(),
            origin,
            primary_size: (primary_width, primary_height),
            size: (width as u32, height as u32),
            indicator: None,
            hints: Vec::new(),
            hint_colors: HintColors {
                border: Color::rgba(0, 255, 0, 0xe0),
                fill: Color::rgba(0, 255, 0, 0x18),
                readability: Color::rgba(0x40, 0x40, 0x40, 0xe6),
                text: Color::rgba(255, 255, 255, 255),
                highlight: Color::rgba(255, 193, 7, 255),
            },
            hints_hidden: false,
            pulse: None,
            temporarily_hidden: false,
            visible: false,
        };
        let mut message: MSG = unsafe { zeroed() };
        unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) };
        Ok(overlay)
    }

    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn set_indicator(&mut self, label: Option<String>) -> io::Result<()> {
        if self.indicator == label {
            return Ok(());
        }
        self.indicator = label;
        self.redraw()
    }

    pub fn set_hints(
        &mut self,
        hints: Vec<OverlayHint>,
        hidden: bool,
        colors: HintColors,
    ) -> io::Result<()> {
        if self.hints == hints && self.hints_hidden == hidden && self.hint_colors == colors {
            return Ok(());
        }
        self.hints = hints;
        self.hints_hidden = hidden;
        self.hint_colors = colors;
        self.redraw()
    }

    pub fn clear_hints(&mut self) -> io::Result<()> {
        self.hints.clear();
        self.hints_hidden = false;
        self.redraw()
    }

    pub fn hide_for_capture(&mut self) {
        self.temporarily_hidden = true;
        if self.visible {
            unsafe { ShowWindow(self.hwnd, SW_HIDE) };
            self.visible = false;
        }
    }

    pub fn show_after_capture(&mut self) -> io::Result<()> {
        self.temporarily_hidden = false;
        self.redraw()
    }

    pub fn locate_cursor(&mut self) -> io::Result<()> {
        let mut point = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.pulse = Some((
            (point.x - self.origin.0, point.y - self.origin.1),
            Instant::now(),
        ));
        self.redraw()
    }

    pub fn click_at(&mut self, x: i32, y: i32, right: bool) -> io::Result<()> {
        Self::move_cursor_to(x, y)?;
        let (down, up) = if right {
            (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
        } else {
            (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
        };
        let inputs = [
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: down,
                        dwExtraInfo: INPUT_MARKER,
                        ..unsafe { zeroed() }
                    },
                },
            },
            INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: up,
                        dwExtraInfo: INPUT_MARKER,
                        ..unsafe { zeroed() }
                    },
                },
            },
        ];
        if unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        } != inputs.len() as u32
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn move_cursor_to(x: i32, y: i32) -> io::Result<()> {
        let origin_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
        let origin_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
        let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
        if width <= 0 || height <= 0 {
            return Err(io::Error::other(
                "Windows reported an invalid virtual screen size",
            ));
        }
        let normalized_x = ((i64::from(x) - i64::from(origin_x)) * 65_535
            / i64::from((width - 1).max(1)))
        .clamp(0, 65_535) as i32;
        let normalized_y = ((i64::from(y) - i64::from(origin_y)) * 65_535
            / i64::from((height - 1).max(1)))
        .clamp(0, 65_535) as i32;
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: normalized_x,
                    dy: normalized_y,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    dwExtraInfo: INPUT_MARKER,
                    ..unsafe { zeroed() }
                },
            },
        };
        if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } != 1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn set_indicator_state(
        &mut self,
        active: bool,
        keys: &[KeyCode],
        speed: SpeedMode,
        locked: bool,
    ) -> io::Result<()> {
        let label = if !active {
            None
        } else {
            let mut label = if locked {
                "HOLD".to_owned()
            } else {
                "fievel".to_owned()
            };
            if !keys.is_empty() {
                label.push_str("  ");
                label.push_str(
                    &keys
                        .iter()
                        .map(|key| {
                            let name = format!("{key:?}");
                            name.strip_prefix("KEY_")
                                .unwrap_or(&name)
                                .to_ascii_uppercase()
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            match speed {
                SpeedMode::Normal => {}
                SpeedMode::Fast => label.push_str("  FAST"),
                SpeedMode::SuperFast => label.push_str("  SUPER FAST"),
                SpeedMode::Slow => label.push_str("  SLOW"),
            }
            Some(label)
        };
        self.set_indicator(label)
    }

    pub fn pump(&mut self) -> io::Result<()> {
        let mut message: MSG = unsafe { zeroed() };
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == 0x0012 {
                break;
            }
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if self
            .pulse
            .is_some_and(|(_, start)| start.elapsed() >= CURSOR_PULSE_DURATION)
        {
            self.pulse = None;
            self.redraw()?;
        }
        Ok(())
    }

    fn redraw(&mut self) -> io::Result<()> {
        if self.temporarily_hidden {
            return Ok(());
        }
        let pixel_count = (self.size.0 as usize)
            .checked_mul(self.size.1 as usize)
            .ok_or_else(|| io::Error::other("virtual screen is too large"))?;
        let pixels =
            unsafe { std::slice::from_raw_parts_mut(self.pixels, pixel_count.saturating_mul(4)) };
        let mut canvas = Canvas::new(pixels, self.size.0, self.size.1);
        canvas.clear(Color::rgba(0, 0, 0, 0));
        if !self.hints_hidden {
            for hint in &self.hints {
                if hint.width <= 0 || hint.height <= 0 {
                    continue;
                }
                let x = hint.x - self.origin.0;
                let y = hint.y - self.origin.1;
                if hint.debug_dot {
                    canvas.fill_rect(
                        x,
                        y,
                        hint.width,
                        hint.height,
                        hint.fill.unwrap_or(self.hint_colors.fill),
                    );
                    continue;
                }
                canvas.fill_rect(
                    x,
                    y,
                    hint.width,
                    hint.height,
                    hint.fill.unwrap_or(self.hint_colors.fill),
                );
                canvas.stroke_rect(
                    x,
                    y,
                    hint.width,
                    hint.height,
                    hint.border.unwrap_or(self.hint_colors.border),
                );
                if hint.label.is_empty() {
                    continue;
                }
                let text_width = Canvas::text_size(&hint.label, 2).0;
                let label_width = (text_width + 12).max(24);
                let label_height = 20;
                canvas.fill_rect(
                    x,
                    y,
                    label_width,
                    label_height,
                    self.hint_colors.readability,
                );
                let prefix_length = hint.prefix_length.min(hint.label.chars().count());
                let prefix: String = hint.label.chars().take(prefix_length).collect();
                let suffix: String = hint.label.chars().skip(prefix_length).collect();
                canvas.draw_text(x + 6, y + 5, &prefix, 2, self.hint_colors.highlight);
                let prefix_width = Canvas::text_size(&prefix, 2).0;
                canvas.draw_text(
                    x + 6 + prefix_width,
                    y + 5,
                    &suffix,
                    2,
                    self.hint_colors.text,
                );
            }
        }
        if let Some(label) = &self.indicator {
            let text_width = Canvas::text_size(label, 2).0;
            let width = (text_width + 20).max(88);
            let height = 30;
            let x = 12i32.saturating_sub(self.origin.0);
            let y = self
                .primary_size
                .1
                .saturating_sub(height + 12)
                .saturating_sub(self.origin.1);
            canvas.fill_rect(x, y, width, height, Color::rgba(26, 26, 26, 0xc8));
            canvas.draw_text(
                x + 10,
                y + height - 4,
                label,
                2,
                Color::rgba(255, 255, 255, 255),
            );
        }
        if let Some(((x, y), started)) = self.pulse {
            if started.elapsed() < CURSOR_PULSE_DURATION {
                let elapsed = started.elapsed().as_secs_f32() / CURSOR_PULSE_DURATION.as_secs_f32();
                let alpha = ((1.0 - elapsed) * 230.0) as u8;
                for (radius, thickness) in [(48, 4), (32, 4), (16, 3)] {
                    canvas.stroke_circle(x, y, radius, thickness, Color::rgba(255, 32, 32, alpha));
                }
            }
        }
        let has_content = self.indicator.is_some()
            || (!self.hints.is_empty() && !self.hints_hidden)
            || self
                .pulse
                .is_some_and(|(_, started)| started.elapsed() < CURSOR_PULSE_DURATION);
        if !has_content {
            if self.visible {
                unsafe { ShowWindow(self.hwnd, SW_HIDE) };
                self.visible = false;
            }
            return Ok(());
        }
        let destination = POINT {
            x: self.origin.0,
            y: self.origin.1,
        };
        let source = POINT { x: 0, y: 0 };
        let size = windows_sys::Win32::Foundation::SIZE {
            cx: self.size.0 as i32,
            cy: self.size.1 as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        if unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                self.screen_dc,
                &destination,
                &size,
                self.memory_dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !self.visible {
            unsafe {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    self.origin.0,
                    self.origin.1,
                    self.size.0 as i32,
                    self.size.1 as i32,
                    SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
                );
            }
            self.visible = true;
        }
        Ok(())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            if !self.memory_dc.is_null() {
                SelectObject(self.memory_dc, self.previous_bitmap);
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                DeleteDC(self.memory_dc);
            }
            if !self.screen_dc.is_null() {
                ReleaseDC(null_mut(), self.screen_dc);
            }
        }
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
