//! Minimal ANSI colouring.
//!
//! Colour is emitted only when the target stream is a terminal, `NO_COLOR` is
//! unset (or empty) and `TERM` is not `dumb`. This replaces the `termion`
//! dependency, which was only ever used for five escape codes.

use std::io::IsTerminal;
use std::sync::OnceLock;

pub const GREEN: &str = "\x1b[38;5;10m";
pub const CYAN: &str = "\x1b[38;5;14m";
pub const RED: &str = "\x1b[38;5;9m";
pub const YELLOW: &str = "\x1b[38;5;11m";
const RESET: &str = "\x1b[m";

#[derive(Clone, Copy)]
pub enum Stream {
    Stdout,
    Stderr,
}

fn env_allows_colour() -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let dumb = std::env::var_os("TERM").is_some_and(|v| v == "dumb");
    !no_color && !dumb
}

pub fn enabled(stream: Stream) -> bool {
    static OUT: OnceLock<bool> = OnceLock::new();
    static ERR: OnceLock<bool> = OnceLock::new();
    match stream {
        Stream::Stdout => {
            *OUT.get_or_init(|| env_allows_colour() && std::io::stdout().is_terminal())
        }
        Stream::Stderr => {
            *ERR.get_or_init(|| env_allows_colour() && std::io::stderr().is_terminal())
        }
    }
}

/// Wrap `text` in `colour` if `stream` should be coloured.
pub fn paint(stream: Stream, colour: &str, text: &str) -> String {
    if enabled(stream) {
        format!("{colour}{text}{RESET}")
    } else {
        text.to_string()
    }
}
