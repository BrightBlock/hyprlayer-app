//! Thoughts library indexer: scan the thoughts tree(s), watch for changes,
//! answer filtered queries and full-text searches.

pub mod indexer;
pub mod roots;
pub mod search;
pub mod watcher;

pub use indexer::*;
pub use roots::*;
