fn main() {
    if let Err(e) = voicevox_cli::cli::run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
