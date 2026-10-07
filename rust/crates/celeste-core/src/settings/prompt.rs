//! Port of `ScarletSettings`'s interactive-input methods
//! (`requireInput`/`requireConfirmYesNo`/`requireSelect`).
//!
//! Java picks a Swing dialog when a display is available and otherwise
//! falls back to console I/O. Rust has no Swing, so [`UserPrompt`] is the
//! seam: this module ports the console fallback path faithfully (plain
//! stdin/stdout, nothing Java-specific about it), and a graphical
//! implementation belongs in `celeste-ui` once its toolkit is chosen (see
//! PORTING.md) -- it implements the same trait, so callers (settings code,
//! future Discord command handlers needing user confirmation) don't care
//! which one is wired in.

use std::io::Write;

pub trait UserPrompt: Send + Sync {
    /// Prompts for a line of text. `sensitive` hides input where the
    /// implementation can (a password field in a GUI; unechoed input on a
    /// real terminal). Returns `None` if the user cancelled.
    fn require_input(&self, display: &str, sensitive: bool) -> Option<String>;

    /// Prompts for a yes/no confirmation.
    fn require_confirm_yes_no(&self, message: &str, title: &str) -> bool;

    /// Prompts the user to pick one of `options`, defaulting the initial
    /// selection to `initial` (if present in `options`).
    fn require_select(&self, message: &str, title: &str, options: &[String], initial: Option<&str>) -> Option<String>;
}

/// Port of the headless fallback path: plain stdin/stdout, no dialog.
pub struct ConsolePrompt;

impl UserPrompt for ConsolePrompt {
    fn require_input(&self, display: &str, _sensitive: bool) -> Option<String> {
        // Java distinguishes a real interactive Console (which can suppress
        // echo for passwords via `readPassword`) from a plain redirected
        // stdin (`Scanner`). Rust's std has no portable "is this a real
        // TTY, and can I read a password without echo" primitive without
        // an extra crate, so this always echoes -- a real terminal-aware
        // implementation (e.g. via the `rpassword` crate) is a reasonable
        // future addition, not attempted here to avoid a speculative
        // dependency with no caller yet.
        print!("{display}: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
            Err(_) => None,
        }
    }

    fn require_confirm_yes_no(&self, message: &str, title: &str) -> bool {
        print!("{title}\n{message} (y/n): ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return false;
        }
        line.trim().eq_ignore_ascii_case("y")
    }

    fn require_select(&self, message: &str, title: &str, options: &[String], initial: Option<&str>) -> Option<String> {
        let mut prompt = format!("{title}\n");
        for (i, opt) in options.iter().enumerate() {
            prompt.push_str(&format!("{i}: {opt}\n"));
        }
        prompt.push_str(message);
        prompt.push_str(": ");
        print!("{prompt}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return initial.map(str::to_string);
        }
        match line.trim().parse::<usize>() {
            Ok(idx) => options.get(idx).cloned(),
            Err(_) => initial.map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_prompt_is_object_safe_and_constructible() {
        // The main thing worth testing here without faking stdin is that
        // this compiles as a trait object -- the real behavior needs a TTY
        // or piped stdin, which isn't worth faking in a unit test.
        let _prompt: Box<dyn UserPrompt> = Box::new(ConsolePrompt);
    }
}
