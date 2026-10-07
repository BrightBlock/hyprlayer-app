#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // ssh runs this binary as its SSH_ASKPASS.
    if let Some(code) = hyprlayer_app_lib::ssh::askpass_helper() {
        std::process::exit(code);
    }
    hyprlayer_app_lib::run()
}
