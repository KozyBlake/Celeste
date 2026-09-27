//! Port of the headless-vs-graphical decision in `IScarletUI.create`
//! (`Platform.forceHeadlessUi() || GraphicsEnvironment.isHeadless()`).
//!
//! Rust has no `GraphicsEnvironment.isHeadless()` equivalent (that's a JVM
//! AWT concept: it probes for a usable display and caches the answer). The
//! closest practical equivalent on Linux is checking whether a display
//! server is reachable at all (`DISPLAY`/`WAYLAND_DISPLAY`); other
//! platforms are assumed to have a display unless explicitly overridden.
//! `SCARLET_HEADLESS` is this port's equivalent of the Java version's
//! `Platform.forceHeadlessUi()` explicit override.

pub fn is_headless() -> bool {
    if let Ok(value) = std::env::var("SCARLET_HEADLESS") {
        if value == "1" || value.eq_ignore_ascii_case("true") {
            return true;
        }
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_env_var_forces_headless() {
        std::env::set_var("SCARLET_HEADLESS", "1");
        assert!(is_headless());
        std::env::remove_var("SCARLET_HEADLESS");
    }
}
