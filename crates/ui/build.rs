//! Embeds the application icon into the Windows executable.

#[path = "../../build-support/windows_icon.rs"]
mod windows_icon;

fn main() {
    windows_icon::embed();
}
