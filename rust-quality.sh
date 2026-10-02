# Sourced by the Nix preBuild hook, with errexit enabled.
cargo fmt --all -- --check
cargo clippy --offline --locked --release --all-targets --all-features -- -D warnings
