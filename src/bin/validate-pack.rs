use std::{env, process::ExitCode};

use desktop_pet_chatgpt::media::Pack;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(root) = args.next().filter(|_| args.next().is_none()) else {
        eprintln!("Usage: validate-pack <pack-directory>");
        return ExitCode::FAILURE;
    };
    match Pack::open(root.as_ref()).and_then(|pack| {
        pack.validate_payloads()?;
        println!("Validated {}", pack.manifest().pack_id);
        Ok(())
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Invalid pack: {error}");
            ExitCode::FAILURE
        }
    }
}
