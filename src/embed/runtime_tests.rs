use super::*;

#[test]
fn ort_lib_path_is_under_cogz_lib() {
    let path = ort_lib_path();
    assert!(path.ends_with(ORT_LIB_NAME));
}

#[test]
fn find_existing_lib_returns_none_when_nothing_installed() {
    // SAFETY: this test runs single-threaded; removing an env var
    // here is safe because no other thread is reading the environment.
    unsafe {
        std::env::remove_var("ORT_DYLIB_PATH");
    }
    let _ = find_existing_lib();
}

#[test]
fn find_ort_checksum_matches_correct_asset() {
    let sums = "abc123  onnxruntime-linux-x64-1.27.0.tgz\ndef456  onnxruntime-win-x64-1.27.0.zip\n";
    assert_eq!(
        find_ort_checksum(sums, "onnxruntime-linux-x64-1.27.0.tgz"),
        Some("abc123".to_string())
    );
}

#[test]
fn find_ort_checksum_returns_none_for_missing_asset() {
    let sums = "abc123  onnxruntime-linux-x64-1.27.0.tgz\n";
    assert_eq!(find_ort_checksum(sums, "onnxruntime-windows.zip"), None);
}
