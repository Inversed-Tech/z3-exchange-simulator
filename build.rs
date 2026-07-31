// Embeds the git commit this binary was actually compiled from, at compile time.
//
// `git rev-parse HEAD` run at *runtime* (the previous approach) reports the
// working tree's current commit, not the commit the running binary was built
// from — if the binary isn't rebuilt after a later commit, the manifest
// silently mislabels every run with a commit whose code never executed. See
// docs/regtest-funding-plan.md and the manifest.rs `read_simulator_commit`
// call site for the failure this caused.
fn main() {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=SIMULATOR_GIT_COMMIT={commit}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
}
