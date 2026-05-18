use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: {} <string> <file>", args[0]);
        process::exit(1);
    }

    if let Err(e) = std::fs::write(&args[2], format!("{}\n", &args[1])) {
        eprintln!("Failed to write to file: {}", e);
    }
}
