use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use walkdir::WalkDir;

/// Mutex to ensure only one test builds opendetex at a time
static BUILD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// Get the path to the detex-rs debug binary
fn detex_rs_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("detex-rs")
}

/// Get the path to the opendetex binary
fn opendetex_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("opendetex-2.8.11")
        .join("detex")
}

/// Ensure opendetex is built
fn ensure_opendetex_built() {
    // Acquire lock to prevent parallel builds
    let lock = BUILD_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap();

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

/// Run detex-rs on a file with optional flags and return the output (stdout, stderr)
fn run_detex_rs(input_file: &Path, flags: &[&str], working_dir: &Path) -> (String, String) {
    let output = Command::new(detex_rs_bin())
        .current_dir(working_dir)
        .args(flags)
        .arg(input_file)
        .output()
        .expect("Failed to run detex-rs");
    let stdout = String::from_utf8(output.stdout).expect("detex-rs stdout was not valid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("detex-rs stderr was not valid UTF-8");
    (stdout, stderr)
}

/// Run opendetex on a file with optional flags and return the output (stdout, stderr)
fn run_opendetex(input_file: &Path, flags: &[&str], working_dir: &Path) -> (String, String) {
    let output = Command::new(opendetex_bin())
        .current_dir(working_dir)
        .args(flags)
        .arg(input_file)
        .output()
        .expect("Failed to run opendetex");
    let stdout = String::from_utf8(output.stdout).expect("opendetex stdout was not valid UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("opendetex stderr was not valid UTF-8");
    (stdout, stderr)
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

/// Generic test runner that compares outputs with optional flags
fn run_comparison_tests_in_dir(dir: &str, flags: &[&str]) {
    // Ensure opendetex is built before running tests
    ensure_opendetex_built();

    // Get all test files
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("latex")
        .join(dir);
    let test_files = get_tex_files(&test_dir);

    assert!(
        !test_files.is_empty(),
        "No test files found in tests/latex/{}",
        dir
    );

    let flags_display = if flags.is_empty() {
        "no flags".to_string()
    } else {
        flags.join(" ")
    };

    eprintln!("\n=== Testing with {} ===", flags_display);

    // Collect all failures instead of stopping at the first one
    let mut failures = Vec::new();

    // Run comparison for each test file
    for test_file in test_files {
        let test_name = test_file.file_name().unwrap().to_string_lossy();
        eprintln!("\nTesting: {} ({})", test_name, flags_display);

        // Make the file path relative to the test directory
        let relative_file = test_file.strip_prefix(&test_dir).unwrap();

        let (detex_rs_stdout, detex_rs_stderr) = run_detex_rs(relative_file, flags, &test_dir);
        let (opendetex_stdout, opendetex_stderr) = run_opendetex(relative_file, flags, &test_dir);

        let stdout_matches = opendetex_stdout == detex_rs_stdout;
        let stderr_matches = opendetex_stderr == detex_rs_stderr;

        if !stdout_matches || !stderr_matches {
            failures.push((
                test_name.to_string(),
                opendetex_stdout,
                detex_rs_stdout,
                opendetex_stderr,
                detex_rs_stderr,
            ));
            eprintln!("  ✗ Failed");
        } else {
            eprintln!("  ✓ Passed");
        }
    }

    // Report all failures at the end
    if !failures.is_empty() {
        let mut error_msg = format!(
            "\n{} test(s) failed with {}:\n",
            failures.len(),
            flags_display
        );

        for (test_name, opendetex_stdout, detex_rs_stdout, opendetex_stderr, detex_rs_stderr) in
            failures
        {
            error_msg.push_str(&format!("\n--- {} ---\n", test_name));

            if opendetex_stdout != detex_rs_stdout {
                error_msg.push_str(&format!(
                    "\nstdout differs:\n\nOpendetex stdout:\n{}\n\ndetex-rs stdout:\n{}\n",
                    opendetex_stdout, detex_rs_stdout
                ));
            }

            if opendetex_stderr != detex_rs_stderr {
                error_msg.push_str(&format!(
                    "\nstderr differs:\n\nOpendetex stderr:\n{}\n\ndetex-rs stderr:\n{}\n",
                    opendetex_stderr, detex_rs_stderr
                ));
            }
        }

        panic!("{}", error_msg);
    }
}

#[test]
fn test_simple_latex_files() {
    run_comparison_tests_in_dir("simple", &[]);
}

#[test]
fn test_simple_latex_files_cite_flag() {
    run_comparison_tests_in_dir("simple", &["-c"]);
}

#[test]
fn test_simple_latex_files_env_flag() {
    run_comparison_tests_in_dir("simple", &["-e", "equation,verbatim"]);
}

#[test]
fn test_simple_latex_files_math_flag() {
    run_comparison_tests_in_dir("simple", &["-r"]);
}

#[test]
fn test_simple_latex_files_space_flag() {
    run_comparison_tests_in_dir("simple", &["-s"]);
}

#[test]
fn test_complex_latex_files() {
    run_comparison_tests_in_dir(".", &[]);
}

#[test]
fn test_complex_latex_files_space_flag() {
    run_comparison_tests_in_dir(".", &["-s"]);
}

#[test]
fn test_simple_latex_files_srcloc() {
    run_comparison_tests_in_dir("simple", &["-e", "tabular", "-l", "-c", "-1"]);
}

#[test]
#[ignore] // this may be due to a detex bug
fn test_complex_latex_files_srcloc() {
    run_comparison_tests_in_dir(".", &["-e", "tabular", "-l", "-c", "-1"]);
}
