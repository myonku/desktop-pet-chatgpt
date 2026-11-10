fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    let result = desktop_pet_chatgpt::run();
    #[cfg(not(windows))]
    let result: Result<(), Box<dyn std::error::Error>> =
        Err("the transparent-window prototype requires Windows".into());

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Desktop Pet ChatGPT: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
