//! `pawprint`: the CLI and the badge (SPEC §9). On Windows this is the
//! console-subsystem executable, and the badge runs as `pawprintw.exe`.

fn main() -> std::process::ExitCode {
    pawprint::main()
}
