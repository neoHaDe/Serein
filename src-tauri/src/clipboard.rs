//! Native Windows clipboard. WebView2 navigator.clipboard often fails silently.

const CF_UNICODETEXT: u32 = 13;

#[cfg(windows)]
pub fn write_text(text: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{HANDLE, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len() * 2;

    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| e.to_string())?;
        if let Err(e) = EmptyClipboard() {
            let _ = CloseClipboard();
            return Err(e.to_string());
        }
        let hmem = match GlobalAlloc(GMEM_MOVEABLE, bytes) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseClipboard();
                return Err(e.to_string());
            }
        };
        let ptr = GlobalLock(hmem) as *mut u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        let _ = GlobalUnlock(hmem);
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, HANDLE(hmem.0)) {
            let _ = CloseClipboard();
            return Err(e.to_string());
        }
        let _ = CloseClipboard();
    }
    Ok(())
}

#[cfg(windows)]
pub fn read_text() -> Result<String, String> {
    use windows::Win32::Foundation::{HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        OpenClipboard(HWND::default()).map_err(|e| e.to_string())?;
        let h = match GetClipboardData(CF_UNICODETEXT) {
            Ok(h) => h,
            Err(e) => {
                let _ = CloseClipboard();
                return Err(e.to_string());
            }
        };
        if h.0.is_null() {
            let _ = CloseClipboard();
            return Ok(String::new());
        }
        let hmem = HGLOBAL(h.0);
        let ptr = GlobalLock(hmem) as *const u16;
        if ptr.is_null() {
            let _ = CloseClipboard();
            return Err("GlobalLock failed".into());
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        let out = String::from_utf16_lossy(slice);
        let _ = GlobalUnlock(hmem);
        let _ = CloseClipboard();
        Ok(out)
    }
}

#[cfg(not(windows))]
pub fn write_text(_text: &str) -> Result<(), String> {
    Err("clipboard: Windows only".into())
}

#[cfg(not(windows))]
pub fn read_text() -> Result<String, String> {
    Err("clipboard: Windows only".into())
}
