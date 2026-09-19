//! The desktop's log file: rotated rather than deleted, UTC timestamps with
//! milliseconds so its lines sort against a server log from another machine.

use log::LevelFilter;
use tauri_plugin_log::{Builder, RotationStrategy};

const MAX_FILE_BYTES: u128 = 5 * 1024 * 1024;
const FILES_KEPT: usize = 5;

pub fn plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let stamp = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    Builder::new()
        .level(level())
        .max_file_size(MAX_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(FILES_KEPT))
        .format(move |out, message, record| {
            let now = time::OffsetDateTime::now_utc()
                .format(&stamp)
                .unwrap_or_default();
            out.finish(format_args!(
                "{now} {:<5} {}: {message}",
                record.level(),
                record.target()
            ))
        })
        .build()
}

/// The leading level of `HYPRLAYER_LOG`, which a server this app spawns also
/// inherits and reads as a full filter.
fn level() -> LevelFilter {
    std::env::var(hyprlayer_server::logging::LEVEL_ENV)
        .ok()
        .and_then(|directive| directive.split(',').next()?.trim().parse().ok())
        .unwrap_or(LevelFilter::Info)
}

/// Panics otherwise reach only stderr, which a packaged app discards.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("{info}\n{}", std::backtrace::Backtrace::force_capture());
        previous(info);
    }));
}
