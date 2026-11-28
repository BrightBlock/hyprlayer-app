#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hyprlayer_client::hosts::Target;
use hyprlayer_desktop::config::{Config, Launch, USAGE};
use hyprlayer_desktop::io::Io;
use hyprlayer_desktop::single_instance::{self, Instance};
use hyprlayer_desktop::{logging, shell};

fn main() {
    // ssh runs this binary as its SSH_ASKPASS; that must answer before anything else starts.
    if let Some(code) = hyprlayer_client::ssh::askpass_helper() {
        std::process::exit(code);
    }
    let launch = Launch::parse(std::env::args().skip(1)).unwrap_or_else(|e| {
        eprintln!("{e}\n{USAGE}");
        std::process::exit(2);
    });
    let identifier = launch.identifier();
    let Some(paths) = launch.paths() else {
        eprintln!("cannot resolve this platform's app dirs for {identifier}");
        std::process::exit(1);
    };
    if let Err(e) = std::fs::create_dir_all(&paths.log) {
        eprintln!("cannot create {}: {e}", paths.log.display());
        std::process::exit(1);
    }
    logging::init(&paths.log);
    let config = Config::new(identifier, paths, launch.attach.map(Target::LocalProfile))
        .unwrap_or_else(|e| {
            log::error!("cannot prepare the profile dirs: {e}");
            std::process::exit(1);
        });
    log::info!(
        "hyprlayer-native starting: pid {}, {}, {} build, {}/{}, protocol {}, server: {}, data {}, logs {}",
        std::process::id(),
        config.identifier,
        if cfg!(debug_assertions) { "debug" } else { "release" },
        std::env::consts::OS,
        std::env::consts::ARCH,
        hyprlayer_proto::PROTOCOL_VERSION,
        config.target.name(),
        config.paths.data.display(),
        config.paths.log.display(),
    );
    let guard = match single_instance::acquire(&config.paths.data) {
        Ok(Instance::First(guard)) => guard,
        Ok(Instance::Second) => {
            log::info!("another client holds this profile; asked it to come forward");
            return;
        }
        Err(e) => {
            log::error!("cannot take this profile's instance lock: {e}");
            std::process::exit(1);
        }
    };
    shell::run(config, Io::start(), guard);
}
