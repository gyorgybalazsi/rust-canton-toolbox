use clap::Parser;
use codegen::codegen::record_struct::generate_rust_code_from_dars;

/// Generate Rust types from DAML DAR files.
#[derive(Parser)]
#[command(
    name = "daml-codegen",
    about = "Generate Rust types from DAML DAR files"
)]
struct Cli {
    /// One or more DAR files to process
    #[arg(long = "dar", required = true, num_args = 1..)]
    dar_paths: Vec<String>,

    /// Output Rust file path
    #[arg(long = "output", short = 'o')]
    output: String,
}

fn main() {
    let cli = Cli::parse();
    let dar_refs: Vec<&str> = cli.dar_paths.iter().map(|s| s.as_str()).collect();

    match generate_rust_code_from_dars(&dar_refs, &cli.output) {
        Ok(()) => {
            eprintln!("Generated {}", cli.output);
        }
        Err(e) => {
            eprintln!("Error: {:#}", e);
            std::process::exit(1);
        }
    }
}
