use std::{env, process::ExitCode};

use desktop_pet_chatgpt::media::{Pack, Variant};

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1).peekable();
    let distribution = args.next_if(|arg| arg == "--distribution").is_some();
    let Some(root) = args.next().filter(|_| args.next().is_none()) else {
        eprintln!("Usage: validate-pack [--distribution] <pack-directory>");
        return ExitCode::FAILURE;
    };
    match Pack::open(root.as_ref()).and_then(|pack| {
        pack.validate_payloads()?;
        if distribution {
            for (index, page) in pack.manifest().pages.iter().enumerate() {
                pack.load_page(index, Variant::Png, || false)?;
                if page.rgba8.is_some() {
                    pack.load_page(index, Variant::Rgba8, || false)?;
                }
                if page.bc7.is_some() {
                    pack.load_page(index, Variant::Bc7, || false)?;
                }
            }
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
