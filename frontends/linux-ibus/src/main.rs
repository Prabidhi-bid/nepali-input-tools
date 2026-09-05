//! `xlit-ibus` — the Linux input method, as an IBus engine.
//!
//! Started by `ibus-daemon` from the component file in `data/xlit.xml`, which
//! names this binary and the engine it provides. On start we connect to the bus
//! the daemon is listening on, claim our well-known name, and serve a factory
//! that hands out one engine object per input context.
//!
//! Nothing here is shared with the Windows text service beyond the three
//! portable crates (`xlit-core`, `xlit-dict`, `xlit-learn`) — the dictionary,
//! the ranking, the verb forms and the vowel folding are all the same code. Only
//! the plumbing differs, and it differs completely: TSF is an in-process COM
//! server loaded into every application, IBus is one program talking D-Bus.

//! Non-Unix targets get a stub `main` rather than being excluded from the
//! workspace, so `cargo check --workspace` on a Windows development machine
//! still type-checks everything else and this crate's absence is explicit.

#[cfg(unix)]
mod engine;
#[cfg(unix)]
mod ibus;
#[cfg(unix)]
mod session;

#[cfg(not(unix))]
fn main() {
    eprintln!("xlit-ibus is the Linux (IBus) frontend; build it on Linux.");
    std::process::exit(1);
}

#[cfg(unix)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(unix)]
use zbus::connection::Builder;
#[cfg(unix)]
use zbus::{interface, zvariant::ObjectPath};

#[cfg(unix)]
use session::XlitEngine;

#[cfg(unix)]
/// The bus name in the component file. Must match `<name>` there.
const BUS_NAME: &str = "org.freedesktop.IBus.Xlit";
#[cfg(unix)]
const FACTORY_PATH: &str = "/org/freedesktop/IBus/Factory";

#[cfg(unix)]
/// Hands out one engine object per input context.
struct Factory {
    next: AtomicU32,
}

#[cfg(unix)]
#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    /// IBus asks for an engine by name and expects the path of a fresh object.
    /// The counter only has to be unique within this process.
    async fn create_engine(
        &self,
        name: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<ObjectPath<'_>> {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        let path = format!("/org/freedesktop/IBus/Engine/{n}");
        let owned = ObjectPath::try_from(path.clone())
            .map_err(|e| zbus::fdo::Error::Failed(format!("bad object path: {e}")))?
            .into_owned();
        server
            .at(&owned, XlitEngine::new())
            .await
            .map_err(|e| zbus::fdo::Error::Failed(format!("cannot serve engine: {e}")))?;
        eprintln!("xlit: created engine {name} at {path}");
        Ok(owned)
    }
}

#[cfg(unix)]
/// Where the IBus daemon is listening.
///
/// `IBUS_ADDRESS` is set when the daemon starts us itself, which is the normal
/// case. The socket file is the fallback for running by hand: it is keyed by
/// machine id and display, and holds `IBUS_ADDRESS=...` among other lines.
fn bus_address() -> Result<String, String> {
    if let Ok(addr) = std::env::var("IBUS_ADDRESS") {
        if !addr.is_empty() {
            return Ok(addr);
        }
    }

    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let machine = std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map_err(|e| format!("cannot read the machine id: {e}"))?;
    let machine = machine.trim();

    // Wayland and X11 name the socket differently, and the display number is
    // part of it. Rather than reconstruct the exact name, scan the directory —
    // there is normally one file, and matching on the machine id is enough.
    let dir = format!("{home}/.config/ibus/bus");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {dir}: {e}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(machine) {
            continue;
        }
        let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
        for line in body.lines() {
            if let Some(addr) = line.strip_prefix("IBUS_ADDRESS=") {
                return Ok(addr.trim().to_string());
            }
        }
    }
    Err(format!("no IBus socket for this machine in {dir} — is ibus-daemon running?"))
}

#[cfg(unix)]
#[tokio::main]
async fn main() {
    // `--ibus` / `-i` is how the daemon launches an engine. We behave the same
    // either way; accepting it keeps the component file conventional.
    let by_daemon = std::env::args().any(|a| a == "--ibus" || a == "-i");

    let address = match bus_address() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("xlit: {e}");
            std::process::exit(1);
        }
    };

    let conn = match Builder::address(address.as_str())
        .and_then(|b| {
            b.name(BUS_NAME)?
                .serve_at(FACTORY_PATH, Factory { next: AtomicU32::new(0) })
        })
        .map(|b| b.build())
    {
        Ok(fut) => match fut.await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("xlit: cannot connect to IBus at {address}: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("xlit: cannot set up the connection: {e}");
            std::process::exit(1);
        }
    };

    eprintln!(
        "xlit: connected to IBus as {BUS_NAME} (started {})",
        if by_daemon { "by the daemon" } else { "by hand" }
    );

    // Nothing else to do on this thread: the object server answers on its own.
    // Exit on Ctrl-C so running it by hand for debugging is not a nuisance.
    let _ = tokio::signal::ctrl_c().await;
    drop(conn);
}
