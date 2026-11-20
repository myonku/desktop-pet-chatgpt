use std::{env, process::ExitCode};

use desktop_pet_chatgpt::{core::Behavior, media::Pack};

const USAGE: &str =
    "Usage: validate-pack --behaviors | [--playback | --distribution] <pack-directory>";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1).peekable();
    if args.next_if(|arg| arg == "--behaviors").is_some() {
        if args.next().is_some() {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
        for behavior in Behavior::ALL {
            println!("{}", behavior.key());
        }
        return ExitCode::SUCCESS;
    }
    let distribution = args.next_if(|arg| arg == "--distribution").is_some();
    let playback = !distribution && args.next_if(|arg| arg == "--playback").is_some();
    let Some(root) = args.next().filter(|_| args.next().is_none()) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    match Pack::open(root.as_ref()).and_then(|pack| {
        if playback || distribution {
            pack.validate_playback()?;
        } else {
            pack.validate_payloads()?;
        }
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
