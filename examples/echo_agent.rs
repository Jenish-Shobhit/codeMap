//! A stand-in "agent" for codeMorph's live tests: copy it to a file named
//! `claude` so herdr recognises the foreground process, then everything typed
//! into its pane is echoed back.

use std::io::{Read, Write};

fn main() {
    let mut buf = [0u8; 4096];
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    loop {
        match input.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut out = stdout.lock();
                let _ = out.write_all(b"agent got: ");
                let _ = out.write_all(&buf[..n]);
                let _ = out.flush();
            }
        }
    }
}
