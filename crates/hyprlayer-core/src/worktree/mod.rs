mod manager;
pub mod seed;
pub mod thoughts_links;

pub use manager::*;

pub const THOUGHTS_DIR: &str = "thoughts";

pub fn is_thoughts_path(path: &str) -> bool {
    path.strip_prefix(THOUGHTS_DIR)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}
