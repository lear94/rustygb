//! Desktop entry point; an empty stub on `wasm32`.

#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    let Err(error) = native::run();
    eprintln!("RustyGB: {error}");
    std::process::ExitCode::FAILURE
}

#[cfg(target_arch = "wasm32")]
fn main() {}
