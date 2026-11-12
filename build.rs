//! Значок и сведения о версии в .exe для Windows.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "ADB Toolkit");
        res.set("FileDescription", "ADB Toolkit");
        if let Err(e) = res.compile() {
            println!("cargo:warning=значок для .exe не встроен: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
