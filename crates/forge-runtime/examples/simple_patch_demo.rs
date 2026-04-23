//! Simple Patch Demo
//!
//! The runtime still lives in the binary crate, so this example acts as a
//! documented entry point instead of linking internal modules directly.

fn main() {
    println!("Simple patch demo");
    println!(
        "Run `cargo run --bin forge_bootstrap -- \"Create a hello.txt file with hello world\" 5 stub`"
    );
}
