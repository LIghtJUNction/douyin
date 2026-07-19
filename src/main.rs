use std::process::ExitCode;

fn main() -> ExitCode {
    match douyin_cli::cli::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("错误: {error}");
            ExitCode::FAILURE
        }
    }
}
