fn write_shot() {
    let _ = std::env::var_os("HOME");
    let _ = std::fs::write("/home/ann/genos-debug/shot.png", b"png");
}
