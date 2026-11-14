//! Asking questions on the terminal, including one whose answer must not be shown.

use std::io::{BufRead, Write};

/// True when a person is at the keyboard, as opposed to input piped from a file or script.
pub fn interactive() -> bool {
    // SAFETY: `isatty` only inspects the file descriptor.
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

fn read_line() -> Result<String, String> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|error| format!("Could not read your answer: {error}"))?;
    Ok(line.trim().to_string())
}

pub fn ask(label: &str) -> Result<String, String> {
    if interactive() {
        eprint!("{label}: ");
        let _ = std::io::stderr().flush();
    }
    read_line()
}

/// Asks for a secret with the terminal's echo off, so it never appears on screen or in
/// scrollback. Piped input is read as it is.
pub fn ask_hidden(label: &str) -> Result<String, String> {
    if !interactive() {
        return read_line();
    }
    eprint!("{label}: ");
    let _ = std::io::stderr().flush();
    // SAFETY: the termios calls read and write the settings of this process's own terminal, into
    // and from a struct on this stack.
    let saved = unsafe {
        let mut settings: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(libc::STDIN_FILENO, &mut settings) != 0 {
            return Err("Could not hide what you type, so the key was not asked for.".to_string());
        }
        let saved = settings;
        settings.c_lflag &= !libc::ECHO;
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &settings);
        saved
    };
    let answer = read_line();
    // SAFETY: as above; this puts the terminal back as it was.
    unsafe {
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved);
    }
    eprintln!();
    answer
}
