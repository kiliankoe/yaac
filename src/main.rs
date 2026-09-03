mod cli;
mod config;
mod notes;
mod output;
mod session;
mod sync;

fn main() -> std::process::ExitCode {
    cli::run()
}
