//! Named production guards for lighting and render picture gates.
//!
//! A shipping build never enables the `knockout` feature, so [`guard`] is
//! always true. Tests that prove a gate actually grades the production
//! behaviour run with `--features knockout` and `GENOS_KNOCKOUT` set to the
//! guard they are removing. The gate must then go red.

/// Include direct lamp and sun light in the presented compose.
pub const DIRECT_IN_COMPOSE: &str = "direct_in_compose";

/// Include bounce (indirect) light in the presented compose.
pub const BOUNCE_IN_COMPOSE: &str = "bounce_in_compose";

/// True when the named production guard is in force.
#[must_use]
#[inline]
pub fn guard(name: &str) -> bool {
    #[cfg(not(feature = "knockout"))]
    {
        let _ = name;
        true
    }
    #[cfg(feature = "knockout")]
    {
        !knocked_out(name)
    }
}

#[cfg(feature = "knockout")]
fn knocked_out(name: &str) -> bool {
    knockout_list().iter().any(|item| item == name)
}

/// Comma-separated guard names listed in `GENOS_KNOCKOUT`.
#[cfg(feature = "knockout")]
#[allow(
    clippy::disallowed_methods,
    reason = "test-only feature: CI sets GENOS_KNOCKOUT for one cargo test process"
)]
fn knockout_list() -> Vec<String> {
    match std::env::var("GENOS_KNOCKOUT") {
        Ok(value) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_named_guard_stays_on_in_the_shipping_build() {
        assert!(super::guard(super::DIRECT_IN_COMPOSE));
        assert!(super::guard(super::BOUNCE_IN_COMPOSE));
        assert!(super::guard("unknown_guard"));
    }
}
