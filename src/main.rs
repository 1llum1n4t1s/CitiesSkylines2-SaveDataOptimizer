#![cfg_attr(not(test), windows_subsystem = "windows")]

mod windows;

fn main() {
    windows::run(std::env::args_os().nth(1).map(std::path::PathBuf::from));
}
