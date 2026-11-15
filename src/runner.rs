//! Spawns `tdl dl` and turns its terminal output into events for the UI.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;

pub enum Event {
    /// One cleaned-up line of tdl output (stdout or stderr).
    Line(String),
    Exited { success: bool, code: Option<i32> },
}

/// Handle to a running tdl process. Dropping it kills the process.
pub struct Job {
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // Don't leave an invisible tdl running after the window closes.
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

pub fn spawn(
    urls: &[String],
    dir: &str,
    ctx: egui::Context,
) -> std::io::Result<(Job, Receiver<Event>)> {
    let mut cmd = Command::new("tdl");
    cmd.arg("dl")
        .arg("-d")
        .arg(dir)
        // Resume a previous interrupted run instead of prompting on stdin.
        .arg("--continue");
    for url in urls {
        cmd.arg("-u").arg(url);
    }
    cmd.env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            std::io::Error::new(e.kind(), "`tdl` was not found in PATH")
        } else {
            e
        }
    })?;

    let (tx, rx) = mpsc::channel();
    let readers = [
        child
            .stdout
            .take()
            .map(|out| spawn_reader(out, tx.clone(), ctx.clone())),
        child
            .stderr
            .take()
            .map(|err| spawn_reader(err, tx.clone(), ctx.clone())),
    ];

    let child = Arc::new(Mutex::new(Some(child)));
    let waiter_child = Arc::clone(&child);
    thread::spawn(move || {
        // The pipes close when tdl exits (or is killed), ending the readers.
        for reader in readers.into_iter().flatten() {
            let _ = reader.join();
        }
        let status = waiter_child
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
            .and_then(|mut child| child.wait().ok());
        let _ = tx.send(Event::Exited {
            success: status.is_some_and(|s| s.success()),
            code: status.and_then(|s| s.code()),
        });
        ctx.request_repaint();
    });

    Ok((
        Job {
            child,
            cancelled: Arc::new(AtomicBool::new(false)),
        },
        rx,
    ))
}

/// Reads a pipe and emits a line at every `\n` *or* `\r`, since progress bars
/// redraw in place using carriage returns / cursor movement.
fn spawn_reader<R: Read + Send + 'static>(
    mut pipe: R,
    tx: Sender<Event>,
    ctx: egui::Context,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        let flush = |pending: &mut Vec<u8>| -> bool {
            let text = strip_ansi(&String::from_utf8_lossy(pending));
            pending.clear();
            let text = text.trim();
            text.is_empty() || tx.send(Event::Line(text.to_owned())).is_ok()
        };
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &byte in &buf[..n] {
                if byte == b'\n' || byte == b'\r' {
                    if !flush(&mut pending) {
                        return;
                    }
                } else {
                    pending.push(byte);
                }
            }
            ctx.request_repaint();
        }
        flush(&mut pending);
        ctx.request_repaint();
    })
}

/// Removes ANSI escape sequences (colors, cursor movement, line erase).
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            // CSI: ESC [ params... final-byte(0x40..=0x7E)
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL  or  ESC ] ... ESC \
            Some(']') => {
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{07}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            // Two-character escape.
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum LineKind {
    Progress { key: String, fraction: f32 },
    Done { key: String },
    Failed { key: String },
    Other,
}

/// tdl renders one tracker line per file, shaped like
/// `<label> ... 42.10% [###.....] [1.2MB in 3s; ~ETA: 5s; 400KB/s]`,
/// ending in `... done! [...]` or `... fail! [...]` when the file finishes.
pub fn classify(line: &str) -> LineKind {
    let Some((label, tail)) = line.rsplit_once(" ... ") else {
        return LineKind::Other;
    };
    let key = label.trim().to_owned();
    let tail = tail.trim_start();
    if key.is_empty() {
        return LineKind::Other;
    }
    if tail.starts_with("done!") {
        return LineKind::Done { key };
    }
    if tail.starts_with("fail!") {
        return LineKind::Failed { key };
    }
    let Some((number, _)) = tail.split_once('%') else {
        return LineKind::Other;
    };
    match number.trim().parse::<f32>() {
        Ok(pct) if pct.is_finite() => LineKind::Progress {
            key,
            fraction: (pct / 100.0).clamp(0.0, 1.0),
        },
        _ => LineKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escape_sequences() {
        assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1A\u{1b}[32mhello\u{1b}[0m"), "hello");
    }

    #[test]
    fn classifies_tracker_lines() {
        assert_eq!(
            classify("Chan(1):2 -> a.mp4 ... 42.50% [####....] [1MB in 3s; ~ETA: 5s]"),
            LineKind::Progress {
                key: "Chan(1):2 -> a.mp4".into(),
                fraction: 0.425
            }
        );
        assert_eq!(
            classify("Chan(1):2 -> a.mp4 ... done! [10MB in 9s; 1MB/s]"),
            LineKind::Done {
                key: "Chan(1):2 -> a.mp4".into()
            }
        );
        assert_eq!(
            classify("Chan(1):2 -> a.mp4 ... fail! [0B in 1s]"),
            LineKind::Failed {
                key: "Chan(1):2 -> a.mp4".into()
            }
        );
        assert_eq!(classify("Error: not authorized"), LineKind::Other);
    }
}
