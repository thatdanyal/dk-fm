// Windows: embed the icon and the "DK.FM" name/version (shown in Explorer, Task Manager and
// Apps & features) into the .exe.
fn main() {
    // what's new in this version: the release commit's subject (it's also the release notes),
    // shown once after updating
    let notes = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=DKFM_NOTES={notes}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "DK.FM")
            .set("ProductName", "DK.FM")
            .set("CompanyName", "thatdanyal")
            .set("OriginalFilename", "DK.FM.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
}
