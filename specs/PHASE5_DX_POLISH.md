# Phase 5: Developer Experience Polish — Implementation Spec

**Repo:** https://github.com/Purabhh/FastRust
**Branch:** main
**Crate:** rust_squared (path: `rust_squared/`), cargo-rsq (path: `cargo-rsq/`)

---

## Prerequisites

```bash
git pull origin main
cargo check --workspace   # must pass before starting
cargo test --workspace    # must pass before starting
```

---

## TASK 5.1: Improve OpenAPI 3.1 Spec Generation

### Goal
Expand schema coverage and improve the generated OpenAPI spec quality.

### Files to modify
- `rust_squared/src/schema.rs` — add missing RsqSchema impls
- `rust_squared/src/openapi.rs` — add error responses, improve param types
- `rust_squared_macros/src/lib.rs` — already handles Vec, Option, HashMap in derive macro (verify)

### Step 1: Add missing RsqSchema impls to `schema.rs`

The existing impls cover: `String`, `bool`, `u64`, `i64`, `f64`. Add these:

```rust
impl RsqSchema for u32 {
    fn schema_name() -> &'static str { "u32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint32" })
    }
}

impl RsqSchema for i32 {
    fn schema_name() -> &'static str { "i32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int32" })
    }
}

impl RsqSchema for f32 {
    fn schema_name() -> &'static str { "f32" }
    fn schema() -> Value {
        serde_json::json!({ "type": "number", "format": "float" })
    }
}

impl RsqSchema for u16 {
    fn schema_name() -> &'static str { "u16" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint16" })
    }
}

impl RsqSchema for i16 {
    fn schema_name() -> &'static str { "i16" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int16" })
    }
}

impl RsqSchema for u8 {
    fn schema_name() -> &'static str { "u8" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "uint8" })
    }
}

impl RsqSchema for i8 {
    fn schema_name() -> &'static str { "i8" }
    fn schema() -> Value {
        serde_json::json!({ "type": "integer", "format": "int8" })
    }
}

impl<T: RsqSchema> RsqSchema for Vec<T> {
    fn schema_name() -> &'static str { "Vec" }
    fn schema() -> Value {
        serde_json::json!({ "type": "array", "items": T::schema() })
    }
}

impl<T: RsqSchema> RsqSchema for Option<T> {
    fn schema_name() -> &'static str { "Option" }
    fn schema() -> Value {
        let mut s = T::schema();
        if let Some(obj) = s.as_object_mut() {
            obj.insert("nullable".to_string(), serde_json::json!(true));
        }
        s
    }
}

impl<T: RsqSchema> RsqSchema for std::collections::HashMap<String, T> {
    fn schema_name() -> &'static str { "HashMap" }
    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": T::schema()
        })
    }
}
```

### Step 2: Add error responses to OpenAPI spec

In `rust_squared/src/openapi.rs`, the current code only generates a `"200"` response. Add standard error responses after the 200.

Find the line that inserts `"responses"` into the operation map. Currently it looks like:
```rust
operation.insert("responses".into(), json!({ "200": success_response }));
```

Change to:
```rust
let mut responses = serde_json::Map::new();
responses.insert("200".into(), success_response);
responses.insert("400".into(), json!({ "description": "Bad Request" }));
responses.insert("401".into(), json!({ "description": "Unauthorized" }));
responses.insert("404".into(), json!({ "description": "Not Found" }));
responses.insert("500".into(), json!({ "description": "Internal Server Error" }));
operation.insert("responses".into(), Value::Object(responses));
```

### Step 3: Add description field to RsqSchema trait

Modify the trait in `schema.rs`:
```rust
pub trait RsqSchema {
    fn schema_name() -> &'static str;
    fn schema() -> Value;
    fn description() -> Option<&'static str> { None }
}
```

This is a backwards-compatible addition (default impl returns None). No existing code breaks.

### Tests (add to schema.rs `#[cfg(test)] mod tests`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_schema() {
        assert_eq!(String::schema(), serde_json::json!({"type": "string"}));
    }

    #[test]
    fn u32_schema() {
        assert_eq!(u32::schema(), serde_json::json!({"type": "integer", "format": "uint32"}));
    }

    #[test]
    fn i32_schema() {
        assert_eq!(i32::schema(), serde_json::json!({"type": "integer", "format": "int32"}));
    }

    #[test]
    fn f32_schema() {
        assert_eq!(f32::schema(), serde_json::json!({"type": "number", "format": "float"}));
    }

    #[test]
    fn vec_schema() {
        assert_eq!(<Vec<String>>::schema(), serde_json::json!({"type": "array", "items": {"type": "string"}}));
    }

    #[test]
    fn option_schema() {
        let s = <Option<i64>>::schema();
        assert_eq!(s["type"], "integer");
        assert_eq!(s["nullable"], true);
    }

    #[test]
    fn hashmap_schema() {
        use std::collections::HashMap;
        let s = <HashMap<String, bool>>::schema();
        assert_eq!(s["type"], "object");
        assert_eq!(s["additionalProperties"]["type"], "boolean");
    }
}
```

---

## TASK 5.2: Implement cargo-rsq CLI

### Goal
Replace the placeholder `cargo-rsq/src/main.rs` with a real CLI using clap.

### Files to modify
- `cargo-rsq/Cargo.toml` — add clap dependency
- `cargo-rsq/src/main.rs` — implement CLI

### Step 1: Add dependency to `cargo-rsq/Cargo.toml`
```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
```

### Step 2: Implement `cargo-rsq/src/main.rs`

```rust
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

    // Create directory structure
    fs::create_dir_all(project_dir.join("src")).expect("failed to create project directory");

    // Write Cargo.toml
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

    // Write src/main.rs
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

    // Check if cargo-watch is installed
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
```

### Tests (add to bottom of main.rs or a separate test)

At minimum, verify the CLI parses args without panicking:

```rust
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
```

---

## TASK 5.3: Hot Reload Dev Command

This is already included in TASK 5.2 as the `dev` subcommand. No additional work needed — it's the `cmd_dev()` function above. Mark as done when 5.2 is complete.

---

## TASK 5.4: Better Error Messages

### Goal
Add `#[track_caller]` to error factory methods, add `.context()` method, and add `source` field.

### File to modify
- `rust_squared/src/error.rs`

### Current state of error.rs
```rust
#[derive(Debug, Clone)]
pub struct RsqError {
    status: StatusCode,
    message: String,
}
```

### Changes

1. **Add `source` field and `context` field:**

```rust
#[derive(Debug)]
pub struct RsqError {
    status: StatusCode,
    message: String,
    context: Option<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
```

Note: Remove `Clone` derive since `Box<dyn Error>` is not Clone. If Clone is needed elsewhere, implement it manually by dropping the source:

```rust
impl Clone for RsqError {
    fn clone(&self) -> Self {
        Self {
            status: self.status,
            message: self.message.clone(),
            context: self.context.clone(),
            source: None, // source is not cloneable
        }
    }
}
```

2. **Add `#[track_caller]` to all factory methods:**

```rust
#[track_caller]
pub fn internal(message: impl Into<String>) -> Self {
    Self::new(message.into(), StatusCode::INTERNAL_SERVER_ERROR)
}

#[track_caller]
pub fn not_found(message: impl Into<String>) -> Self {
    Self::new(message.into(), StatusCode::NOT_FOUND)
}
// ... same for bad_request, method_not_allowed, unsupported_media_type, unprocessable_entity
```

3. **Add `.context()` method:**

```rust
pub fn context(mut self, msg: impl Into<String>) -> Self {
    self.context = Some(msg.into());
    self
}
```

4. **Add `.with_source()` method:**

```rust
pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
    self.source = Some(Box::new(source));
    self
}
```

5. **Implement `std::error::Error::source()`:**

```rust
impl std::error::Error for RsqError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}
```

6. **Update `Display` to include context:**

```rust
impl Display for RsqError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.context {
            Some(ctx) => write!(f, "{}: {} ({})", self.status, self.message, ctx),
            None => write!(f, "{}: {}", self.status, self.message),
        }
    }
}
```

7. **Update `new()` constructor:**

```rust
#[track_caller]
pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
    Self {
        status,
        message: message.into(),
        context: None,
        source: None,
    }
}
```

### IMPORTANT: Check all call sites

Search the codebase for `RsqError::` and `.clone()` on RsqError to make sure nothing breaks. The main concern is removing `Clone` from the derive — if any code clones an RsqError, the manual Clone impl handles it (drops source).

### Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;

    #[test]
    fn factory_methods_return_correct_status() {
        assert_eq!(RsqError::internal("err").status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(RsqError::not_found("err").status(), StatusCode::NOT_FOUND);
        assert_eq!(RsqError::bad_request("err").status(), StatusCode::BAD_REQUEST);
        assert_eq!(RsqError::method_not_allowed("err").status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(RsqError::unsupported_media_type("err").status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(RsqError::unprocessable_entity("err").status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn context_appended_to_display() {
        let err = RsqError::internal("something failed").context("while parsing config");
        assert!(err.to_string().contains("while parsing config"));
    }

    #[test]
    fn with_source_chains_errors() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err = RsqError::internal("read failed").with_source(io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn clone_drops_source() {
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "oops");
        let err = RsqError::internal("fail").with_source(io_err);
        let cloned = err.clone();
        assert!(cloned.source().is_none());
        assert_eq!(cloned.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
```

---

## Quality Gates (ALL must pass before pushing)

1. `cargo check --workspace` — zero errors
2. `cargo test --workspace` — all existing tests + new tests pass
3. `cargo clippy --workspace` — no new warnings
4. Each task committed separately with descriptive message
5. Push: `git push https://<YOUR_GITHUB_PAT>@github.com/Purabhh/FastRust.git main`

---

## Dependency Summary

| Crate | Version | Where | Why |
|-------|---------|-------|-----|
| clap | 4 (features: derive) | cargo-rsq/Cargo.toml | CLI argument parsing |
