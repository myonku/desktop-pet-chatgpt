use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use std::{env, path::Path};

    use desktop_pet_chatgpt::integrations::chatgpt::read_account;

    let mut args = env::args_os().skip(1);
    let Some(cli) = args.next().filter(|_| args.next().is_none()) else {
        eprintln!("Usage: check-chatgpt-account <absolute-path-to-trusted-codex.exe>");
        return ExitCode::FAILURE;
    };
    match read_account(Path::new(&cli)) {
        Ok(snapshot) => {
            println!("{}", serde_json::json!(snapshot));
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", serde_json::json!({"error": error}));
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("check-chatgpt-account currently requires Windows");
    ExitCode::FAILURE
}
