//! Developer-only, explicit Android Binder probe. Never prints provider material.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let token_context = args.first().is_some_and(|arg| arg == "--token-context");
    if token_context {
        args.remove(0);
    }
    if args.len() > 1 {
        eprintln!("Usage: guard-adb-probe [--token-context] [serial]");
        std::process::exit(1);
    }
    let serial = args.first().map(String::as_str);
    let result = if token_context {
        douyin_cli::guard_adb::verify_token_key_context(serial)
    } else {
        douyin_cli::guard_adb::verify_fixture(serial)
    };
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
