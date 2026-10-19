fn write_shot(out: &std::path::Path) {
    let dir = out.join("genos-debug/shots");
    let _ = std::fs::create_dir_all(&dir);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR");
    let _ = runtime;
}

fn upload(words: &[u32]) -> Result<u32, std::num::TryFromIntError> {
    u32::try_from(words.len())
}
