//! The live MCP bridge: a running Blobr listens on
//! `$XDG_RUNTIME_DIR/app/dev.jamsdesign.Blobr/blobr.sock` (see
//! `blobr_mcp::live`). A thread accepts connections from `blobr-mcp`; each call is handed to
//! the app as a [`Message::Agent`] and runs on the open document like any other edit, so it
//! shows up at once and lands in History as an undo step.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc::Sender;
use jd_mcp::ToolResult;
use serde_json::Value;

use crate::app::Message;

/// What the listener tells the app.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// An agent connected (`true`) or went away.
    Connected(bool),
    Call { name: String, args: Value, reply: mpsc::Sender<ToolResult> },
}

static LISTENING: AtomicBool = AtomicBool::new(false);

/// Hands a message to the app from a plain thread.
fn push(out: &mut Sender<Message>, mut m: Message) -> bool {
    loop {
        match out.try_send(m) {
            Ok(()) => return true,
            Err(e) if e.is_full() => {
                m = e.into_inner();
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
}

/// The subscription's stream: binds the socket (unless another Blobr has it) and forwards
/// calls.
pub fn listen() -> impl Stream<Item = Message> {
    iced::stream::channel(32, async |out: Sender<Message>| {
        if let Some(listener) = blobr_mcp::live::bind() {
            LISTENING.store(true, Ordering::SeqCst);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut out = out.clone();
                    std::thread::spawn(move || {
                        if !push(&mut out, Message::Agent(AgentEvent::Connected(true))) {
                            return;
                        }
                        let mut calls = out.clone();
                        blobr_mcp::live::serve(stream, |name, args| {
                            let (reply, answer) = mpsc::channel();
                            if !push(&mut calls, Message::Agent(AgentEvent::Call { name, args, reply })) {
                                return Err("Blobr is closing".into());
                            }
                            answer.recv_timeout(Duration::from_secs(110)).unwrap_or_else(|_| Err("Blobr didn't answer in time (a dialog may be open)".into()))
                        });
                        push(&mut out, Message::Agent(AgentEvent::Connected(false)));
                    });
                }
            });
        }
        std::future::pending::<()>().await;
    })
}

/// Removes the socket when the app closes, so agents fall back to headless at once.
pub fn cleanup() {
    if LISTENING.swap(false, Ordering::SeqCst)
        && let Some(p) = blobr_mcp::live::socket_path()
    {
        let _ = std::fs::remove_file(p);
    }
}
