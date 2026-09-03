mod cli;
mod output;
mod session;

fn main() -> std::process::ExitCode {
    cli::run()
}
