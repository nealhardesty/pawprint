//! `pawprintw.exe`: the same program as `pawprint.exe`, built for the
//! Windows GUI subsystem so that starting the badge at logon or from
//! `pawprint` never flashes a console window (SPEC §11.1.6). Other systems
//! ship only `pawprint`.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    pawprint::main()
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("pawprintw is the Windows badge executable; use `pawprint` here.");
    std::process::ExitCode::from(2)
}
