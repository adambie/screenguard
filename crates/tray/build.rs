fn main() {
    let git_version = std::process::Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let version = git_version.unwrap_or_else(|| {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string())
    });

    println!("cargo:rustc-env=SCREENGUARD_VERSION={version}");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
}
