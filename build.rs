// Windows: embed the icon and the "DK.FM" name/version (shown in Explorer, Task Manager and
// Apps & features) into the .exe.
fn main() {
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
