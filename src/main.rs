mod cli;
mod config;
mod notes;
mod output;
mod session;

fn main() -> std::process::ExitCode {
    cli::run()
}
