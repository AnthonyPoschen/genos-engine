#![expect(clippy::must_use_candidate, reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md")]

//! Exit status of one check.
//!
//! 0 is a pass. 1 is a fail: the self-test missed, a scored picture failed,
//! or nothing was scored. `--skip-capture` and a missing window are empty
//! comparisons and fail. 2 needs a person. A one-sample compare does not
//! take the pass path: the verdict is already needs-human.

use crate::verdict::Verdict;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitFacts {
    pub self_test_ok: bool,
    /// Worst verdict among pictures that were scored. Pass when nothing was.
    pub scene_verdict: Verdict,
    /// At least one checkpoint was scored.
    pub scored: bool,
    /// Every requested checkpoint was scored.
    pub complete: bool,
}

pub fn exit_code(facts: ExitFacts) -> i32 {
    if !facts.self_test_ok || facts.scene_verdict == Verdict::Fail {
        return 1;
    }
    if facts.scene_verdict == Verdict::NeedsHuman {
        return 2;
    }
    if !facts.scored {
        return 1;
    }
    if !facts.complete {
        return 2;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> ExitFacts {
        ExitFacts {
            self_test_ok: true,
            scene_verdict: Verdict::Pass,
            scored: true,
            complete: true,
        }
    }

    #[test]
    fn exit_codes_match_the_policy() {
        assert_eq!(exit_code(facts()), 0);
        let mut miss = facts();
        miss.self_test_ok = false;
        assert_eq!(exit_code(miss), 1);
        let mut fail = facts();
        fail.scene_verdict = Verdict::Fail;
        assert_eq!(exit_code(fail), 1);
        let mut human = facts();
        human.scene_verdict = Verdict::NeedsHuman;
        assert_eq!(exit_code(human), 2);
        let mut empty = facts();
        empty.scored = false;
        empty.complete = false;
        assert_eq!(exit_code(empty), 1);
        let mut partial = facts();
        partial.complete = false;
        assert_eq!(exit_code(partial), 2);
        let mut one_sample_fail = empty;
        one_sample_fail.scored = true;
        one_sample_fail.scene_verdict = Verdict::Fail;
        assert_eq!(exit_code(one_sample_fail), 1);
    }
}
