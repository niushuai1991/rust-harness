#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("--purge-local-data")])
    {
        match rust_harness_desktop::uninstall::purge_local_data() {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    rust_harness_desktop::run()
}
