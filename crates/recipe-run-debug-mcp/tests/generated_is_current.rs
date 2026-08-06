//! Guard: `src/api/generated.rs` must match what the vendored specs produce.
//!
//! Without this, someone can edit the generated file by hand (or refresh a spec and forget to
//! regenerate), and the mismatch only shows up as a confusing runtime parse error.

use std::process::Command;

#[test]
fn generated_models_match_the_vendored_specs() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let generated_path = format!("{manifest}/src/api/generated.rs");
    let before = std::fs::read_to_string(&generated_path).expect("generated.rs is present");

    let output = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "--example", "generate_models"])
        .current_dir(manifest)
        .output()
        .expect("run the generator");

    assert!(
        output.status.success(),
        "the generator failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = std::fs::read_to_string(&generated_path).expect("generated.rs after regeneration");

    if before != after {
        // Put the file back so a failing test doesn't leave a dirty tree.
        std::fs::write(&generated_path, &before).expect("restore generated.rs");
        panic!(
            "src/api/generated.rs is out of date with specs/. Run:\n\
             \n    cargo run --example generate_models\n\n\
             and commit the result. Do not edit the generated file by hand — adapters belong in \
             src/api/model.rs."
        );
    }
}
