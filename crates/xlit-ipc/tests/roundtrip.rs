//! A real socket, a real server thread, a real client — the framing unit tests
//! only prove the bytes, not that a connection survives more than one exchange.
#![cfg(unix)]

use std::time::{Duration, Instant};

use xlit_ipc::{transport, Client, Request, Response};

/// Unix sockets cap the path at ~108 bytes, so keep it short and unique.
fn endpoint(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("xlit-t{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("s").to_string_lossy().into_owned()
}

fn spawn_server(endpoint: &str) -> std::thread::JoinHandle<()> {
    let listener = transport::bind(endpoint).unwrap();
    std::thread::spawn(move || {
        xlit_ipc::serve(listener, |req| match req {
            Request::Candidates { text, .. } => Response::candidates(vec![xlit_ipc::Candidate {
                text: text.to_uppercase(),
                source: "rule".into(),
                score: 100,
            }]),
            Request::Commit { .. } => Response::ok(),
            Request::Transliterate { text } => Response::text(text),
            Request::Ping | Request::Shutdown => Response::ok(),
        })
        .unwrap();
    })
}

fn wait_until_up(endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if transport::is_running(endpoint) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("server never came up on {endpoint}");
}

#[test]
fn one_connection_carries_many_requests() {
    let ep = endpoint("many");
    let server = spawn_server(&ep);
    wait_until_up(&ep);

    let mut client = Client::connect_at(&ep).unwrap();
    for word in ["namaste", "nepal", "dhanyabaad"] {
        let cands = client.candidates(word, None).unwrap();
        assert_eq!(cands[0].text, word.to_uppercase());
    }
    client.commit("namaste", "नमस्ते").unwrap();
    assert_eq!(client.transliterate("x").unwrap(), "x");

    client.call(&Request::Shutdown).unwrap();
    server.join().unwrap();
}

#[test]
fn a_second_daemon_refuses_to_share_the_socket() {
    let ep = endpoint("dup");
    let server = spawn_server(&ep);
    wait_until_up(&ep);

    let err = match transport::bind(&ep) {
        Ok(_) => panic!("a second bind should have been refused"),
        Err(e) => e,
    };
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);

    Client::connect_at(&ep).unwrap().call(&Request::Shutdown).unwrap();
    server.join().unwrap();
}

#[test]
fn a_socket_left_by_a_killed_daemon_is_reclaimed() {
    let ep = endpoint("stale");
    // Bind and drop without serving: the inode outlives the listener even
    // though nothing is listening any more.
    drop(transport::bind(&ep).unwrap());
    std::fs::write(&ep, b"").ok();
    let listener = match transport::bind(&ep) {
        Ok(l) => l,
        Err(e) => panic!("stale socket should be replaced: {e}"),
    };
    assert_eq!(listener.endpoint(), ep);
}
