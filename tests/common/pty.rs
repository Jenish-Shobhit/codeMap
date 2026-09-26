//! A pty harness shared by the pty and live tests.
#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A minimal terminal screen: enough of VT100 to replay what ratatui and
/// crossterm write (cursor moves, text, clears), ignoring colours.
pub struct Screen {
    cols: usize,
    rows: usize,
    cells: Vec<Vec<char>>,
    x: usize,
    y: usize,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        Screen {
            cols,
            rows,
            cells: vec![vec![' '; cols]; rows],
            x: 0,
            y: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        let s = String::from_utf8_lossy(bytes);
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\x1b' => match it.next() {
                    Some('[') => {
                        let mut params = String::new();
                        let mut fin = ' ';
                        for d in it.by_ref() {
                            if ('@'..='~').contains(&d) {
                                fin = d;
                                break;
                            }
                            params.push(d);
                        }
                        let nums: Vec<usize> = params
                            .trim_start_matches('?')
                            .split(';')
                            .map(|p| p.parse().unwrap_or(0))
                            .collect();
                        match fin {
                            'H' | 'f' => {
                                self.y = nums.first().copied().unwrap_or(1).max(1) - 1;
                                self.x = nums.get(1).copied().unwrap_or(1).max(1) - 1;
                            }
                            'J' if nums.first() == Some(&2) => {
                                self.cells = vec![vec![' '; self.cols]; self.rows];
                            }
                            _ => {}
                        }
                    }
                    Some(']') => {
                        // OSC: skip to BEL or ST
                        while let Some(d) = it.next() {
                            if d == '\x07' {
                                break;
                            }
                            if d == '\x1b' {
                                it.next();
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                '\r' => self.x = 0,
                '\n' => self.y = (self.y + 1).min(self.rows - 1),
                c if c.is_control() => {}
                c => {
                    let w = codemorph::util::char_width(c).max(1);
                    if self.y < self.rows && self.x < self.cols {
                        self.cells[self.y][self.x] = c;
                        if w == 2 && self.x + 1 < self.cols {
                            self.cells[self.y][self.x + 1] = '\0';
                        }
                    }
                    self.x += w;
                }
            }
        }
    }

    pub fn text(&self) -> String {
        self.cells
            .iter()
            .map(|r| {
                r.iter()
                    .filter(|c| **c != '\0')
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct Pty {
    pub child: Child,
    pub writer: File,
    pub out: Arc<Mutex<Vec<u8>>>,
    pub first_byte: Arc<Mutex<Option<Instant>>>,
    pub started: Instant,
    pub cols: u16,
    pub rows: u16,
}

pub fn binary() -> String {
    std::env::var("CODEMORPH_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_codemorph").to_string())
}

pub fn spawn(args: &[&str], envs: &[(&str, &str)], cols: u16, rows: u16) -> Pty {
    let mut master: libc::c_int = 0;
    let mut slave: libc::c_int = 0;
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty fills the two descriptors; we own them afterwards.
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    assert_eq!(rc, 0, "openpty failed");
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env("TERM", "xterm-256color")
        .env_remove("HERDR_ENV")
        .env_remove("HERDR_SOCKET_PATH")
        .env_remove("HERDR_PLUGIN_CONTEXT_JSON")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let started = Instant::now();
    let child = cmd.spawn().expect("spawn codemorph");
    let master = unsafe { File::from_raw_fd(master) };
    let writer = master.try_clone().unwrap();
    let out = Arc::new(Mutex::new(Vec::new()));
    let first_byte = Arc::new(Mutex::new(None));
    {
        let out = out.clone();
        let first_byte = first_byte.clone();
        let mut reader = master;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut fb = first_byte.lock().unwrap();
                        if fb.is_none() {
                            *fb = Some(Instant::now());
                        }
                        out.lock().unwrap().extend_from_slice(&buf[..n]);
                    }
                }
            }
        });
    }
    Pty {
        child,
        writer,
        out,
        first_byte,
        started,
        cols,
        rows,
    }
}

impl Pty {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.out.lock().unwrap()).into_owned()
    }

    /// The screen as the terminal would show it now.
    pub fn screen(&self) -> String {
        let mut sc = Screen::new(self.cols as usize, self.rows as usize);
        sc.feed(&self.out.lock().unwrap());
        sc.text()
    }

    /// Wait until the screen shows `needle` (once `from` bytes have arrived).
    pub fn wait_for(&self, needle: &str, from: usize, timeout: Duration) -> Option<Duration> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.len() > from && self.screen().contains(needle) {
                return Some(self.started.elapsed());
            }
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    pub fn len(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    pub fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(s)) = self.child.try_wait() {
                return Some(s);
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
