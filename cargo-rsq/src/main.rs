use clap::{Parser, Subcommand};
use std::fs;
use std::process::Command;

#[derive(Parser)]
#[command(name = "cargo-rsq", about = "FastRust project toolkit")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new FastRust project
    New {
        /// Project name
        name: String,
    },
    /// Run cargo check with FastRust-specific hints
    Check,
    /// Start dev server with hot reload (requires cargo-watch)
    Dev,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Check => cmd_check(),
        Commands::Dev => cmd_dev(),
    }
}

fn cmd_new(name: &str) {
    let project_dir = std::path::Path::new(name);
    if project_dir.exists() {
        eprintln!("Error: directory '{}' already exists", name);
        std::process::exit(1);
    }

    fs::create_dir_all(project_dir.join("src")).expect("failed to create project directory");

    let cargo_toml = format!(
r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
rust_squared = "0.1"
tokio = {{ version = "1", features = ["macros", "rt-multi-thread", "net"] }}
serde = {{ version = "1", features = ["derive"] }}
"#
    );
    fs::write(project_dir.join("Cargo.toml"), cargo_toml).expect("failed to write Cargo.toml");

    let main_rs = format!(
r#"use rust_squared::RsqApp;

#[tokio::main]
async fn main() {{
    let app = RsqApp::new()
        .get("/", || async {{ Ok("Hello from {name}!") }})
        .expect("route should register");

    println!("Listening on http://127.0.0.1:3000");
    app.serve(([127, 0, 0, 1], 3000).into()).await.unwrap();
}}
"#
    );
    fs::write(project_dir.join("src/main.rs"), main_rs).expect("failed to write main.rs");

    println!("Created FastRust project '{}'", name);
    println!("  cd {}", name);
    println!("  cargo run");
}

fn cmd_check() {
    println!("Running cargo check with FastRust hints...");
    let status = Command::new("cargo")
        .args(["check", "--workspace"])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("\nAll checks passed.");
            println!("Tip: Run `cargo clippy --workspace` for additional lints.");
        }
        Ok(s) => {
            std::process::exit(s.code().unwrap_or(1));
        }
        Err(e) => {
            eprintln!("Failed to run cargo check: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_dev() {
    println!("Starting dev server with hot reload...");

    let check = Command::new("cargo")
        .args(["watch", "--version"])
        .output();

    match check {
        Ok(output) if output.status.success() => {
            let status = Command::new("cargo")
                .args(["watch", "-x", "run"])
                .status()
                .expect("failed to start cargo watch");
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        _ => {
            eprintln!("Error: cargo-watch is not installed.");
            eprintln!("Install it with: cargo install cargo-watch");
            eprintln!("Then retry: cargo rsq dev");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn cli_parses_new_command() {
        let cli = Cli::parse_from(["cargo-rsq", "new", "my_project"]);
        match cli.command {
            Commands::New { name } => assert_eq!(name, "my_project"),
            _ => panic!("expected New command"),
        }
    }

    #[test]
    fn cli_parses_check_command() {
        let cli = Cli::parse_from(["cargo-rsq", "check"]);
        assert!(matches!(cli.command, Commands::Check));
    }

    #[test]
    fn cli_parses_dev_command() {
        let cli = Cli::parse_from(["cargo-rsq", "dev"]);
        assert!(matches!(cli.command, Commands::Dev));
    }
}
