//! Failing example for `std::env::var`, `std::env::var_os`, and `std::thread::sleep`.
//! Library code takes the value from its caller.

pub fn reads_env_and_sleeps() -> bool {
    std::thread::sleep(std::time::Duration::from_millis(0));
    std::env::var("GENOS_EXAMPLE").is_ok() || std::env::var_os("GENOS_EXAMPLE").is_some()
}
