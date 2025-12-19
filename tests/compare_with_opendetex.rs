use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

/// Get the path to the detex-rs debug binary
fn detex_rs_bin() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");
    path.push("debug");
    path.push("detex-rs");
    path
}

/// Get the path to the opendetex binary
fn opendetex_bin() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("opendetex-2.8.11");
    path.push("detex");
    path
}

/// Ensure opendetex is built
fn ensure_opendetex_built() {
    let opendetex_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("opendetex-2.8.11");
    let opendetex_bin = opendetex_bin();

    if !opendetex_bin.exists() {
        eprintln!("Building opendetex...");
        let status = Command::new("make")
            .current_dir(&opendetex_dir)
            .status()
            .expect("Failed to run make for opendetex");

        assert!(status.success(), "Failed to build opendetex");
    }
}

/// Run detex-rs on a file and return the output
fn run_detex_rs(input_file: &Path) -> String {
    let output = Command::new(detex_rs_bin())
        .arg(input_file)
        .output()
        .expect("Failed to run detex-rs");

    String::from_utf8(output.stdout).expect("detex-rs output was not valid UTF-8")
}

/// Run opendetex on a file and return the output
fn run_opendetex(input_file: &Path) -> String {
    let output = Command::new(opendetex_bin())
        .arg(input_file)
        .output()
        .expect("Failed to run opendetex");

    String::from_utf8(output.stdout).expect("opendetex output was not valid UTF-8")
}

/// Get all .tex files in a directory (recursively)
fn get_tex_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("tex"))
        .map(|e| e.path().to_path_buf())
        .collect();

    files.sort();
    files
}

#[test]
fn test_simple_latex_files() {
    // Ensure opendetex is built before running tests
    ensure_opendetex_built();

    // Get all test files
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("latex_tests").join("simple");
    let test_files = get_tex_files(&test_dir);

    assert!(!test_files.is_empty(), "No test files found in latex_tests/simple");

    // Run comparison for each test file
    for test_file in test_files {
        let test_name = test_file.file_name().unwrap().to_string_lossy();
        eprintln!("\nTesting: {}", test_name);

        let detex_rs_output = run_detex_rs(&test_file);
        let opendetex_output = run_opendetex(&test_file);

        assert_eq!(
            opendetex_output,
            detex_rs_output,
            "Output mismatch for {}\n\nOpendetex output:\n{}\n\ndetex-rs output:\n{}",
            test_name,
            opendetex_output,
            detex_rs_output
        );

        eprintln!("  ✓ Passed");
    }
}
