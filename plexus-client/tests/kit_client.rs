//! PERF.115-36 · plexus-wire's conformance kit, its client role, with
//! plexus-client as the client under test: `cargo test -p plexus-client
//! --features ws-native --test kit_client`. Needs python3, and plexus-wire
//! beside this repository (or where `PLEXUS_WIRE` says); without either it
//! fails, naming what is missing. What plexus-client does not do yet is a
//! line of `tests/kit_client.expect`.
//!
//! This test is its own client: the kit runs it once a case, as
//! `kit_client --url URL --service NAME PATH... [--name VALUE]...`, and it
//! makes the one call as a generated client makes it (`Transport::call_raw`,
//! `decode_raw`), printing each event as a line of JSON: a data event as
//! itself, a progress report as `{"progress": …}`, an error as
//! `{"error": …}` with its code, then exit 1. Interrupted, it lets the call
//! go (unsubscribing it) and exits 130.

use futures::StreamExt;
use plexus_client::{decode_raw, ClientError, ClientEvent, MethodRef, Transport, WsTransport};
use serde_json::{json, Map, Value};
use std::time::Duration;

fn usage() -> ! {
    eprintln!("kit_client --url URL --service NAME PATH... [--name VALUE]...");
    std::process::exit(2)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--url") {
        tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime").block_on(client());
    } else {
        kit();
    }
}

/// The kit, its client role, this test the client.
fn kit() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let wire = std::env::var_os("PLEXUS_WIRE").map(std::path::PathBuf::from).unwrap_or_else(|| here.join("../../plexus-wire"));
    let run = wire.join("conformance/run.py");
    assert!(run.exists(), "plexus-wire's kit is not at {} (PLEXUS_WIRE names another)", run.display());
    let me = std::env::current_exe().expect("this test's path");
    let cmd = format!("{} --url {{url}} --service {{backend}} {{path}} {{flags}}", me.display());
    let status = std::process::Command::new("python3")
        .args(["-I"])
        .arg(&run)
        .args(["run", "--role", "client", "--cmd", &cmd, "--expect-fail"])
        .arg(here.join("tests/kit_client.expect"))
        .status()
        .expect("python3 runs the kit");
    assert!(status.success(), "the kit's client role: plexus-client failed a case it does not expect to ({status})");
}

async fn client() {
    let mut args = std::env::args().skip(1).peekable();
    let (mut url, mut service, mut path, mut params) = (None, None, Vec::new(), Map::new());
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--service" => service = args.next(),
            flag if flag.starts_with("--") => {
                let value = args.next().unwrap_or_else(|| usage());
                let value = serde_json::from_str(&value).unwrap_or(Value::String(value));
                params.insert(flag[2..].replace('-', "_"), value);
            }
            segment => path.push(segment.to_owned()),
        }
    }
    let (Some(url), Some(service), Some(method)) = (url, service, path.pop()) else { usage() };
    if path.is_empty() {
        usage();
    }
    let method = MethodRef::new(leak(path.join(".")), leak(method));
    let transport = match WsTransport::connect(&url, &service).await {
        Ok(t) => t,
        Err(e) => {
            println!("{}", json!({"error": {"connect": e.to_string()}}));
            std::process::exit(1)
        }
    };
    let mut events = decode_raw::<Value>(transport.call_raw(method, Value::Object(params)));
    loop {
        tokio::select! {
            event = events.next() => match event {
                None => break,
                Some(Ok(ClientEvent::Data(data))) => println!("{data}"),
                Some(Ok(ClientEvent::Progress { message, percentage })) => {
                    println!("{}", json!({"progress": {"message": message, "percentage": percentage}}))
                }
                Some(Err(e)) => {
                    println!("{}", json!({"error": described(&e)}));
                    std::process::exit(1)
                }
            },
            _ = tokio::signal::ctrl_c() => {
                // Let the call go: dropping its stream unsubscribes it.
                drop(events);
                tokio::time::sleep(Duration::from_millis(300)).await;
                std::process::exit(130)
            }
        }
    }
}

/// An error, with what the kit looks for in it: its kind and its code.
fn described(e: &ClientError) -> Value {
    match e {
        ClientError::Remote { message, code, recoverable } => {
            json!({"remote": message, "code": code, "recoverable": recoverable})
        }
        other => json!({"client": other.to_string(), "debug": format!("{other:?}")}),
    }
}
