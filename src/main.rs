//! Desktop entry point; an empty stub on `wasm32`.

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::run().unwrap_or_else(|error| {
        eprintln!("RustyGB: {error}");
        std::process::ExitCode::FAILURE
    })
}

#[cfg(target_arch = "wasm32")]
fn main() {}
