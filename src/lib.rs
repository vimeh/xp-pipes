//! Shared XP Pipes engine, with native presentation adapters.
mod animation;
mod config;
#[cfg(target_os = "linux")]
mod egl;
mod geometry;
#[cfg(target_os = "macos")]
mod macos;
mod materials;
#[cfg(target_os = "linux")]
mod platform;
#[cfg(test)]
mod reference;
mod renderer;
mod rng;
mod simulation;
mod texture;
mod timing;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Command-line entry point. macOS uses the native saver bundle instead.
#[must_use]
pub fn run_cli() -> std::process::ExitCode {
    use config::{Action, Config, HELP};
    use std::process::ExitCode;
    match Config::parse(std::env::args_os().skip(1)) {
        Ok(Action::Help) => print!("{HELP}"),
        Ok(Action::Version) => println!("wlpipes {}", env!("CARGO_PKG_VERSION")),
        Ok(Action::Run(config)) => {
            if let Err(error) = run(config) {
                eprintln!("wlpipes: {error}");
                return ExitCode::FAILURE;
            }
        }
        Err(error) => {
            eprintln!("wlpipes: {error}\nTry 'wlpipes --help'.");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(target_os = "linux")]
fn run(config: config::Config) -> Result<()> {
    platform::run(config)
}

#[cfg(not(target_os = "linux"))]
fn run(_: config::Config) -> Result<()> {
    Err("use the native XP Pipes.saver bundle on macOS".into())
}
