//! Running external commands with their output in the one log stream.
//!
//! [`run_logged`] is the way regroup runs an external command whose output
//! belongs to the log stream (git, ssh, ...):
//! - normal mode: stdout/stderr are captured and logged line by line through
//!   [`crate::ui::log`] (activity pane in the full-screen ui, stdout in
//!   plain mode);
//! - direct output mode (temporary CLI pass from the TUI): stdin is
//!   inherited and stdout/stderr are forwarded live to our own streams —
//!   interactive prompts work and the output appears in the CLI — and the
//!   output is also recorded into the UI log channel, so the full CLI pass
//!   reappears in the TUI Logs pane when the TUI resumes.

use std::io::{self, Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use super::logging::{is_direct_output_mode, log, record};
use super::types::UiEvent;

/// Run `cmd`, putting its output into the one log stream (see module docs).
pub fn run_logged(cmd: Command) -> io::Result<Output> {
    if is_direct_output_mode() {
        run_live(cmd)
    } else {
        run_captured(cmd)
    }
}

/// Normal mode: capture stdout/stderr and log every line (activity pane in
/// the full-screen ui, stdout in plain mode).
fn run_captured(mut cmd: Command) -> io::Result<Output> {
    let output = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).output()?;
    log_lines(&output.stdout);
    log_lines(&output.stderr);
    Ok(output)
}

/// Direct output mode (temporary CLI pass): stdin inherited, stdout/stderr
/// piped, forwarded live to our own streams by pump threads while being
/// accumulated; the accumulated output is then recorded into the UI log
/// channel — it was already shown live in the CLI, so it must not be
/// printed again.
fn run_live(mut cmd: Command) -> io::Result<Output> {
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut child_out = child.stdout.take().expect("piped stdout");
    let mut child_err = child.stderr.take().expect("piped stderr");
    let out_acc = Arc::new(Mutex::new(Vec::new()));
    let err_acc = Arc::new(Mutex::new(Vec::new()));
    let pump_out = thread::spawn({
        let acc = Arc::clone(&out_acc);
        move || pump(&mut child_out, &mut io::stdout(), &acc)
    });
    let pump_err = thread::spawn({
        let acc = Arc::clone(&err_acc);
        move || pump(&mut child_err, &mut io::stderr(), &acc)
    });
    let status = child.wait()?;
    let _ = pump_out.join();
    let _ = pump_err.join();
    let stdout = std::mem::take(&mut *out_acc.lock().unwrap());
    let stderr = std::mem::take(&mut *err_acc.lock().unwrap());
    record_lines(&stdout);
    record_lines(&stderr);
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Forward everything read from `src` to `sink` immediately (no buffering:
/// prompts must appear live), accumulating it in `acc`.
fn pump(src: &mut impl Read, sink: &mut impl Write, acc: &Mutex<Vec<u8>>) -> io::Result<()> {
    let mut buf = [0u8; 4096];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sink.write_all(&buf[..n])?;
        sink.flush()?;
        acc.lock().unwrap().extend_from_slice(&buf[..n]);
    }
    Ok(())
}

fn log_lines(bytes: &[u8]) {
    for line in String::from_utf8_lossy(bytes).lines() {
        if !line.is_empty() {
            log(line);
        }
    }
}

fn record_lines(bytes: &[u8]) {
    for line in String::from_utf8_lossy(bytes).lines() {
        if !line.is_empty() {
            record(UiEvent::Global(line.to_string()));
        }
    }
}
