//! Simple PTY example - spawn bash and interact with it
//!
//! Run with: cargo run -p cas-mux --example simple_pty

use cas_mux::{Pty, PtyConfig, PtyEvent};
use std::io::{self, Write};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("=== CAS-MUX PTY Prototype ===");
    println!("Spawning bash shell...\n");

    // Spawn a bash PTY
    let config = PtyConfig::default();
    let mut pty = Pty::spawn("test-shell", config)?;

    println!("PTY spawned with ID: {}", pty.id());
    println!("Type commands (they go to bash). Type 'exit' to quit.\n");

    // Spawn a task to read user input and send to PTY
    let writer = pty.writer_handle();
    tokio::spawn(async move {
        let stdin = io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            if stdin.read_line(&mut line).is_err() {
                break;
            }
            let mut w = writer.lock().await;
            if w.write_all(line.as_bytes()).is_err() {
                break;
            }
            let _ = w.flush();
        }
    });

    // Read PTY output and display (raw bytes - pass through to terminal)
    while let Some(event) = pty.recv().await {
        match event {
            PtyEvent::Output(data) => {
                // Pass raw bytes directly to stdout - terminal handles escape sequences
                io::stdout().write_all(&data)?;
                io::stdout().flush()?;
            }
            PtyEvent::Exited(code) => {
                println!("\n[Process exited with code: {code:?}]");
                break;
            }
            PtyEvent::Error(e) => {
                println!("\n[PTY error: {e}]");
                break;
            }
        }
    }

    Ok(())
}
