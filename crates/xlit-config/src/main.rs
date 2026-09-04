//! Word editor for the xlit input method.
//!
//! Teaches the engine words it gets wrong. Type the Latin you want to use, the
//! Devanagari it should produce, and Save; the pair is written to
//! `%APPDATA%\xlit\xlit-learn.json` with enough weight to beat the built-in
//! dictionary outright, and the running input method picks it up on the next
//! word without anything being restarted.
//!
//! A separate process on purpose: the input method is a DLL loaded into every
//! application that takes typing, and putting a window with its own message
//! loop inside all of them is a way to hang somebody else's UI thread.
//!
//! Launched by the `+` on the floating bar, optionally seeded with the word in
//! progress:
//!
//! ```text
//! xlit-config.exe --add kathmandu --suggest काठमाडौं
//! ```

#![cfg(windows)]
#![windows_subsystem = "windows"]

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, GetSysColor, COLOR_BTNFACE, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HFONT,
    OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowTextLengthW,
    GetWindowTextW, LoadCursorW, PostQuitMessage, RegisterClassW, SendMessageW,
    SetWindowTextW, ShowWindow, TranslateMessage, BS_DEFPUSHBUTTON, CW_USEDEFAULT, EN_CHANGE,
    ES_AUTOHSCROLL, IDC_ARROW, LBS_NOTIFY, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, MSG,
    SW_SHOW, WINDOW_EX_STYLE, WM_COMMAND, WM_CLOSE, WM_DESTROY, WM_SETFONT, WNDCLASSW,
    WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;

use xlit_core::Engine;
use xlit_dict::DictRanker;
use xlit_learn::LearnStore;

const ID_LATIN: usize = 101;
const ID_DEVA: usize = 102;
const ID_SAVE: usize = 103;
const ID_LIST: usize = 104;
const ID_REMOVE: usize = 105;
const ID_STATUS: usize = 106;

struct App {
    store: LearnStore,
    engine: Engine,
    latin: HWND,
    deva: HWND,
    list: HWND,
    status: HWND,
    /// Rows currently in the list box, so a selection index maps back to a pair.
    rows: Vec<(String, String)>,
    /// Set once the user edits the Devanagari field themselves, after which we
    /// stop overwriting it with the engine's guess.
    deva_touched: bool,
    /// True while *we* are setting a field, so the resulting EN_CHANGE is not
    /// mistaken for the user typing.
    suppress: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn main() {
    let (seed_latin, seed_deva) = parse_args();
    unsafe { run(seed_latin, seed_deva) };
}

/// `--add <latin> [--suggest <devanagari>]`
fn parse_args() -> (String, String) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut latin = String::new();
    let mut deva = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--add" if i + 1 < args.len() => {
                latin = args[i + 1].clone();
                i += 2;
            }
            "--suggest" if i + 1 < args.len() => {
                deva = args[i + 1].clone();
                i += 2;
            }
            _ => i += 1,
        }
    }
    (latin, deva)
}

fn learn_path() -> std::path::PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("xlit");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("xlit-learn.json")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn text_of(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..got as usize])
    }
}

fn set_text(h: HWND, s: &str) {
    // Bind the buffer: a temporary would be alive for the call, but only by
    // the rule about statement-scoped temporaries, which is easy to break.
    let buf = wide(s);
    unsafe { _ = SetWindowTextW(h, PCWSTR(buf.as_ptr())) };
}

unsafe fn run(seed_latin: String, seed_deva: String) {
    let hinst = GetModuleHandleW(None).unwrap_or_default();
    let class = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hInstance: hinst.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: CreateSolidBrush(windows::Win32::Foundation::COLORREF(GetSysColor(
            COLOR_BTNFACE,
        ))),
        lpszClassName: w!("xlit_config_window"),
        ..Default::default()
    };
    RegisterClassW(&class);

    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("xlit_config_window"),
        w!("xlit - add a word"),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        472,
        520,
        None,
        None,
        Some(hinst.into()),
        None,
    );
    let Ok(hwnd) = hwnd else { return };

    build_controls(hwnd, hinst.into(), &seed_latin, &seed_deva);
    _ = ShowWindow(hwnd, SW_SHOW);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe fn build_controls(
    parent: HWND,
    hinst: windows::Win32::Foundation::HINSTANCE,
    seed_latin: &str,
    seed_deva: &str,
) {
    // Nirmala UI so the Devanagari field and the word list are legible; the
    // default GUI font has no Devanagari at all.
    let font: HFONT = CreateFontW(
        -15,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
        w!("Nirmala UI"),
    );

    let mk = |class: PCWSTR, text: PCWSTR, style: u32, x, y, w, h, id: usize| -> HWND {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            text,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style),
            x,
            y,
            w,
            h,
            Some(parent),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(id as *mut _)),
            Some(hinst),
            None,
        )
        .unwrap_or_default();
        SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        hwnd
    };

    let vis = (WS_CHILD | WS_VISIBLE).0;
    let edit = vis | WS_BORDER.0 | ES_AUTOHSCROLL as u32;

    mk(w!("STATIC"), w!("When I type this (Latin):"), vis, 16, 14, 420, 20, 0);
    let latin = mk(w!("EDIT"), PCWSTR::null(), edit, 16, 36, 420, 28, ID_LATIN);

    mk(w!("STATIC"), w!("...insert this:"), vis, 16, 76, 420, 20, 0);
    let deva = mk(w!("EDIT"), PCWSTR::null(), edit, 16, 98, 420, 28, ID_DEVA);

    mk(w!("BUTTON"), w!("Save word"), vis | BS_DEFPUSHBUTTON as u32, 16, 138, 130, 32, ID_SAVE);
    let status = mk(w!("STATIC"), PCWSTR::null(), vis, 158, 145, 278, 20, ID_STATUS);

    mk(w!("STATIC"), w!("Words you have taught it:"), vis, 16, 188, 420, 20, 0);
    let list = mk(
        w!("LISTBOX"),
        PCWSTR::null(),
        vis | WS_BORDER.0 | WS_VSCROLL.0 | LBS_NOTIFY as u32,
        16,
        210,
        420,
        216,
        ID_LIST,
    );

    mk(w!("BUTTON"), w!("Remove selected"), vis, 16, 438, 150, 32, ID_REMOVE);

    let store = LearnStore::open(learn_path()).unwrap_or_else(|_| LearnStore::in_memory());
    let engine = Engine::nepali().with_ranker(Box::new(DictRanker::builtin()));

    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            store,
            engine,
            latin,
            deva,
            list,
            status,
            rows: Vec::new(),
            deva_touched: !seed_deva.is_empty(),
            suppress: false,
        });
    });

    if !seed_latin.is_empty() {
        set_text(latin, seed_latin);
    }
    if !seed_deva.is_empty() {
        set_text(deva, seed_deva);
    }
    refresh_list();
    _ = SetFocus(Some(if seed_latin.is_empty() { latin } else { deva }));
}

fn refresh_list() {
    APP.with(|a| {
        let mut g = a.borrow_mut();
        let Some(app) = g.as_mut() else { return };
        app.rows = app
            .store
            .entries()
            .into_iter()
            .map(|(i, c, _)| (i, c))
            .collect();
        unsafe {
            SendMessageW(app.list, LB_RESETCONTENT, None, None);
            for (latin, deva) in &app.rows {
                let line = wide(&format!("{latin}   \u{2192}   {deva}"));
                SendMessageW(
                    app.list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(line.as_ptr() as isize)),
                );
            }
        }
    });
}

fn set_status(msg: &str) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            set_text(app.status, msg);
        }
    });
}

/// Fill the Devanagari field with the engine's best guess, until the user
/// edits it themselves.
fn suggest() {
    APP.with(|a| {
        let mut g = a.borrow_mut();
        let Some(app) = g.as_mut() else { return };
        if app.deva_touched {
            return;
        }
        let latin = text_of(app.latin);
        let guess = app
            .engine
            .candidates(&latin)
            .into_iter()
            .next()
            .map(|c| c.text)
            .unwrap_or_default();
        app.suppress = true;
        set_text(app.deva, &guess);
        app.suppress = false;
    });
}

fn save() {
    let done = APP.with(|a| {
        let mut g = a.borrow_mut();
        let Some(app) = g.as_mut() else { return None };
        let latin = text_of(app.latin).trim().to_string();
        let deva = text_of(app.deva).trim().to_string();
        if latin.is_empty() || deva.is_empty() {
            return Some(Err("Fill in both boxes.".to_string()));
        }
        if latin == deva {
            return Some(Err("Those are the same - nothing to teach.".to_string()));
        }
        match app.store.pin(&latin, &deva) {
            Ok(()) => Some(Ok(format!("Saved: {latin} \u{2192} {deva}"))),
            Err(e) => Some(Err(format!("Could not save: {e}"))),
        }
    });
    match done {
        Some(Ok(msg)) => {
            refresh_list();
            set_status(&msg);
        }
        Some(Err(msg)) => set_status(&msg),
        None => {}
    }
}

fn remove_selected() {
    let done = APP.with(|a| {
        let mut g = a.borrow_mut();
        let app = g.as_mut()?;
        let sel = unsafe { SendMessageW(app.list, LB_GETCURSEL, None, None) }.0;
        if sel < 0 {
            return Some(Err("Pick a word from the list first.".to_string()));
        }
        let (latin, deva) = app.rows.get(sel as usize)?.clone();
        match app.store.remove(&latin, &deva) {
            Ok(()) => Some(Ok(format!("Removed {latin} \u{2192} {deva}"))),
            Err(e) => Some(Err(format!("Could not remove: {e}"))),
        }
    });
    match done {
        Some(Ok(msg)) => {
            refresh_list();
            set_status(&msg);
        }
        Some(Err(msg)) => set_status(&msg),
        None => {}
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as usize;
                let note = ((wparam.0 >> 16) & 0xFFFF) as u32;
                match (id, note) {
                    (ID_SAVE, _) => save(),
                    (ID_REMOVE, _) => remove_selected(),
                    (ID_LATIN, n) if n == EN_CHANGE => suggest(),
                    (ID_DEVA, n) if n == EN_CHANGE => {
                        APP.with(|a| {
                            if let Some(app) = a.borrow_mut().as_mut() {
                                if !app.suppress {
                                    app.deva_touched = true;
                                }
                            }
                        });
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
