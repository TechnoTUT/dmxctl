fn main() {
    if let Err(error) = dmxctl::cli::run(std::env::args().skip(1).collect()) {
        eprintln!("{}", serde_json::json!({"error": error.to_string()}));
        std::process::exit(1);
    }
}
