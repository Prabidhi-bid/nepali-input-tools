//! Launching the word editor.
//!
//! The editor is a **separate process** (`xlit-config.exe`), not a dialog put
//! up inside the host application. A text service is loaded into Word, Chrome,
//! Explorer and everything else that takes typing; running a modal window with
//! its own message loop in all of those is a good way to deadlock somebody
//! else's UI thread. A child process cannot.
//!
//! It edits the same `%APPDATA%\xlit\xlit-learn.json` the engine reads, and the
//! engine re-reads it on the next word, so a word added in the editor is usable
//! immediately without restarting anything.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Weak;

use crate::session::Session;

/// `xlit-config.exe`, installed next to the DLL.
///
/// The 32-bit payload lives in an `x86\` subdirectory while the editor sits at
/// the install root, so look one level up as well. In a dev tree both land in
/// `target\<profile>\`.
fn editor_path() -> Option<PathBuf> {
    let dll = PathBuf::from(crate::dll_path());
    let dir = dll.parent()?;
    let candidates = [
        dir.join("xlit-config.exe"),
        dir.parent().map(|p| p.join("xlit-config.exe"))?,
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Open the editor, seeded with the word in progress if there is one.
pub fn open(sess: &Weak<RefCell<Session>>) {
    let seed = sess
        .upgrade()
        .and_then(|s| s.try_borrow().ok().map(|s| (s.buf.clone(), s.preview())));

    let Some(exe) = editor_path() else {
        crate::debug("xlit-config.exe not found next to the DLL");
        return;
    };

    let mut cmd = Command::new(exe);
    if let Some((latin, suggestion)) = seed {
        if !latin.is_empty() {
            cmd.arg("--add").arg(latin).arg("--suggest").arg(suggestion);
        }
    }
    match cmd.spawn() {
        Ok(_) => crate::debug("word editor launched"),
        Err(e) => crate::debug(&format!("could not launch the word editor: {e}")),
    }
}
