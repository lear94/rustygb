//! Diagnostics for the terminal.

use std::fmt::Display;
use std::io::{self, Write};

/// Prints to standard error, ignoring failures: `eprintln!` panics on a closed pipe, which
/// would abort before the game is saved.
pub(super) fn report(message: impl Display) {
    let _ = writeln!(io::stderr(), "{message}");
}
