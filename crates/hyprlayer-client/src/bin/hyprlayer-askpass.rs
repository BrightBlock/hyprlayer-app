//! Stands in for the desktop binary as ssh's askpass helper in tests.

fn main() {
    let code = hyprlayer_client::ssh::askpass_helper().unwrap_or(1);
    std::process::exit(code);
}
