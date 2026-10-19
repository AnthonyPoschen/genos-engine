fn reads_an_unregistered_switch() {
    let _ = std::env::var("GENOS_NOT_A_MIRROR");
}
