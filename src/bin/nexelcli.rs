use std::env;

fn main() {
    match env::consts::OS {
        "macos" => {
            eprintln!("it haven't implemented yet");
        }
        "linux" => {
            eprintln!("it haven't implemented yet");
        }
        "windows" => {
            eprintln!("it haven't implemented yet");
        }
        _ => {
            eprintln!("not supported system {}", env::consts::OS);
        }
    }
}