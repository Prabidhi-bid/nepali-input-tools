# xlit-daemon

One process per user that owns the transliteration engine. Frontends connect to
it and send a Latin buffer, getting ranked candidates back; they hold no engine
state of their own.

## Why

On Linux this is optional — an IBus engine or a Fcitx5 addon is one long-lived
process, and linking `xlit-core` directly is simpler. Windows is the reason it
exists: a TSF text service is loaded into *every* process that accepts text, so
a dictionary held inside the service is a dictionary paid for once per running
application. The daemon holds it once.

## Run

```bash
cargo run -p xlit-daemon
```

```bash
xlit --client namaste
```

| Flag | Meaning |
|------|---------|
| `--socket PATH` | listen somewhere else (`\\.\pipe\...` on Windows) |
| `--data-dir DIR` | keep `xlit-learn.json` in `DIR` |
| `--status` | is a daemon listening? exit 1 if not |
| `--stop` | ask a running daemon to exit |

`$XLIT_SOCKET` overrides the default endpoint for every xlit program, daemon and
client alike.

Default endpoint:

- Linux/macOS — `$XDG_RUNTIME_DIR/xlit/xlit.sock`, mode 0700, in a directory the
  session manager removes at logout. A socket left behind by a daemon that was
  killed rather than stopped is detected (by connecting, not by the file
  existing) and replaced.
- Windows — `\\.\pipe\xlit-daemon-$USERNAME`, with the creating user's default
  ACL, so another desktop user cannot read one's picks or write to one's
  learning store.

Default learning store: `$XDG_DATA_HOME/xlit/xlit-learn.json`, or
`%APPDATA%\xlit\xlit-learn.json` — the same paths the frontends use when they
link the engine directly, so the two never keep separate histories.

## Autostart (systemd user service)

```bash
install -Dm755 target/release/xlit-daemon ~/.local/bin/xlit-daemon
install -Dm644 crates/xlit-daemon/dist/xlit-daemon.service ~/.config/systemd/user/xlit-daemon.service
systemctl --user enable --now xlit-daemon
```

## Protocol

Length-prefixed JSON over the socket: a little-endian `u32` byte count, then the
body. One request, one response; a connection carries as many pairs as the
client likes, so a frontend connects once per session.

```jsonc
{"op":"candidates","text":"namaste"}      // → {"ok":true,"candidates":[{"text":"नमस्ते","source":"confirmed","score":349}, …]}
{"op":"commit","input":"namaste","chosen":"नमस्ते"}  // → {"ok":true}
{"op":"transliterate","text":"namaste"}   // → {"ok":true,"text":"नमस्ते"}
{"op":"ping"}                             // → {"ok":true}
{"op":"shutdown"}                         // → {"ok":true}, then the daemon exits
```

A failure comes back as `{"ok":false,"error":"…"}` and never as a dropped
connection: a frontend that loses the daemon mid-word would have to throw away
the user's composition, so the daemon answers even when it cannot help.
`source` is a plain string rather than an enum so that adding a ranking layer
later cannot break an older client. Frames are capped at 1 MiB.

The client side is `xlit_ipc::Client` — blocking and synchronous on purpose. A
keystroke's round trip over a local socket is microseconds, and an input method
that answered asynchronously would have to keep composition state around to
match replies to the buffer they were asked about.
