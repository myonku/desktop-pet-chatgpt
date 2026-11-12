use std::path::PathBuf;

use super::*;

#[test]
fn run_value_round_trips_in_an_isolated_hkcu_key() {
    let key = format!(
        r"Software\DesktopPetChatGPT.Validation-{}",
        std::process::id()
    );
    let wide: Vec<_> = key.encode_utf16().chain([0]).collect();
    let autostart = Autostart {
        key: PCWSTR(wide.as_ptr()),
    };
    struct Cleanup(PCWSTR);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, self.0) };
        }
    }
    let _cleanup = Cleanup(autostart.key);
    let exe = PathBuf::from(r"C:\桌宠 Programs\desktop-pet-chatgpt.exe");
    let command = startup_command(&exe).unwrap();
    let expected: Vec<_> = format!("\"{}\"\0", exe.display())
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(command, expected);
    assert!(startup_command(Path::new("relative.exe")).is_err());
    assert!(startup_command(&PathBuf::from(format!("C:\\{}.exe", "x".repeat(260)))).is_err());
    assert_eq!(autostart.read().unwrap(), None);
    autostart.write(Some(&command)).unwrap();
    assert_eq!(autostart.read().unwrap(), Some(command));
    autostart.write(None).unwrap();
    autostart.write(None).unwrap();
    assert_eq!(autostart.read().unwrap(), None);
}
