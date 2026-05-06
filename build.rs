fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("res/app.ico");
        res.set("ProductName", "RAM Limiter");
        res.set("FileDescription", "RAM Limiter");
        let _ = res.compile();
    }
}
