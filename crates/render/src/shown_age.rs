//! Age of the shown surface-cache mean.
//!
//! The screen pass tallies near red-box pixels into `gstats.diag[14..26]` when
//! `GENOS_LIGHTING_SC_TRACE` is set. The stamp is cache region 26, past the
//! SH-L1, written when a fine texel's mean is committed. Compose does not read
//! that word.
//!
//! A commit postdates the lamp-move stamp when its frame is at or after the
//! newest ring entry that still reaches the texel. A commit before that stamp
//! predates it: the shown mean was stored before the lamp moved.

// diag[14..26]. 0..13 and the screen-list words are already counters.
/// `diag[14]`. Near red-box pixels tallied this frame.
pub const DIAG_RED: usize = 14;
/// `diag[15]`. No shown commit: no slot (`DIAG_NOSLOT`), or a slot whose
/// mean the committing pass never wrote (seeded, or not yet relit).
pub const DIAG_NEVER: usize = 15;
/// `diag[16]`. Age 0 frames.
pub const DIAG_AGE0: usize = 16;
/// `diag[17]`. Age 1 to 3.
pub const DIAG_AGE1_3: usize = 17;
/// `diag[18]`. Age 4 to 7.
pub const DIAG_AGE4_7: usize = 18;
/// `diag[19]`. Age 8 to 15.
pub const DIAG_AGE8_15: usize = 19;
/// `diag[20]`. Age 16 or more.
pub const DIAG_AGE16: usize = 20;
/// `diag[21]`. Commit frame is before the lamp-move stamp.
pub const DIAG_PREDATES: usize = 21;
/// `diag[22]`. Commit frame is at or after the lamp-move stamp.
pub const DIAG_POSTDATES: usize = 22;
/// `diag[23]`. Sum of ages over pixels that have a commit.
pub const DIAG_SUM_AGE: usize = 23;
/// `diag[24]`. A commit, and no lamp stamp reaches the texel.
pub const DIAG_NOSTAMP: usize = 24;
/// `diag[25]`. The pixel's texel key is not in the cache at all. A subset of `DIAG_NEVER`.
pub const DIAG_NOSLOT: usize = 25;
/// `diag[26]`. The pixel is on a turned surface (flag 2048). It has no fine
/// texel, so the fine-texel counters above do not describe what it shows.
/// Compose reads the world-cache patch. Every such pixel is also in `DIAG_NOSLOT`.
pub const DIAG_SLANTED: usize = 26;
/// `diag[27]`. Slanted pixels whose compose source has a commit.
/// That source is the world-cache patch (25 cm, or the 50 cm parent).
pub const DIAG_FEED_N: usize = 27;
/// `diag[28]`. Sum of those ages.
pub const DIAG_FEED_SUM: usize = 28;
/// `diag[29]`. That commit is before the lamp-move stamp.
pub const DIAG_FEED_PRE: usize = 29;
/// `diag[30]`. That commit is at or after the lamp-move stamp.
pub const DIAG_FEED_POST: usize = 30;
/// `diag[31]`. Slanted pixels whose compose source has no commit stamp.
pub const DIAG_FEED_NONE: usize = 31;
/// `diag[32]`. Slanted pixels whose 25 cm patch (or 50 cm parent) has a commit.
pub const DIAG_PATCH_N: usize = 32;
/// `diag[33]`. Sum of those patch ages.
pub const DIAG_PATCH_SUM: usize = 33;
/// `diag[34]`. The patch commit predates the lamp-move stamp.
pub const DIAG_PATCH_PRE: usize = 34;
/// `diag[35]`. The patch commit postdates the lamp-move stamp.
pub const DIAG_PATCH_POST: usize = 35;
/// `diag[36]`. No 25 cm or 50 cm commit.
pub const DIAG_PATCH_NONE: usize = 36;
/// `diag[37]`. Unused on this branch: compose does not show a separate feeder.
pub const DIAG_FEED_PROBES: usize = 37;

/// How the shown commit sits against the lamp-move stamp.
///
/// The screen tally implements this. The tests lock it. A shipping build
/// does not call it from Rust.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// The texel has never committed a mean.
    Never,
    /// Committed, and no ring entry reaches the texel.
    Unstamped,
    /// Commit frame is before the lamp-move stamp.
    Predates,
    /// Commit frame is at or after the lamp-move stamp.
    Postdates,
}

/// Age in frames, and where the commit sits against the lamp stamp.
///
/// `commit == 0` is never committed. `stamp == 0` is no lamp entry in range.
/// Postdates includes the stamp frame itself.
#[cfg_attr(not(test), allow(dead_code))]
pub fn classify(frame: u32, commit: u32, stamp: u32) -> (Option<u32>, Stamp) {
    if commit == 0 {
        return (None, Stamp::Never);
    }
    let age = frame.saturating_sub(commit);
    let stamp = if stamp == 0 {
        Stamp::Unstamped
    } else if commit >= stamp {
        Stamp::Postdates
    } else {
        Stamp::Predates
    };
    (Some(age), stamp)
}

/// Which age counter a committed texel adds to. Matches the screen tally.
#[cfg_attr(not(test), allow(dead_code))]
pub fn age_slot(age: u32) -> usize {
    if age == 0 {
        DIAG_AGE0
    } else if age <= 3 {
        DIAG_AGE1_3
    } else if age <= 7 {
        DIAG_AGE4_7
    } else if age <= 15 {
        DIAG_AGE8_15
    } else {
        DIAG_AGE16
    }
}

/// One frame of the red-box tally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgeHist {
    pub red: u32,
    pub never: u32,
    pub age0: u32,
    pub age1_3: u32,
    pub age4_7: u32,
    pub age8_15: u32,
    pub age16: u32,
    pub predates: u32,
    pub postdates: u32,
    pub sum_age: u32,
    pub nostamp: u32,
    pub noslot: u32,
    pub slanted: u32,
    pub feed_n: u32,
    pub feed_sum: u32,
    pub feed_pre: u32,
    pub feed_post: u32,
    pub feed_none: u32,
    pub patch_n: u32,
    pub patch_sum: u32,
    pub patch_pre: u32,
    pub patch_post: u32,
    pub patch_none: u32,
    pub feed_probes: u32,
}

impl AgeHist {
    pub fn from_diag(diag: &[u32]) -> Self {
        let at = |i: usize| diag.get(i).copied().unwrap_or(0);
        Self {
            red: at(DIAG_RED),
            never: at(DIAG_NEVER),
            age0: at(DIAG_AGE0),
            age1_3: at(DIAG_AGE1_3),
            age4_7: at(DIAG_AGE4_7),
            age8_15: at(DIAG_AGE8_15),
            age16: at(DIAG_AGE16),
            predates: at(DIAG_PREDATES),
            postdates: at(DIAG_POSTDATES),
            sum_age: at(DIAG_SUM_AGE),
            nostamp: at(DIAG_NOSTAMP),
            noslot: at(DIAG_NOSLOT),
            slanted: at(DIAG_SLANTED),
            feed_n: at(DIAG_FEED_N),
            feed_sum: at(DIAG_FEED_SUM),
            feed_pre: at(DIAG_FEED_PRE),
            feed_post: at(DIAG_FEED_POST),
            feed_none: at(DIAG_FEED_NONE),
            patch_n: at(DIAG_PATCH_N),
            patch_sum: at(DIAG_PATCH_SUM),
            patch_pre: at(DIAG_PATCH_PRE),
            patch_post: at(DIAG_PATCH_POST),
            patch_none: at(DIAG_PATCH_NONE),
            feed_probes: at(DIAG_FEED_PROBES),
        }
    }

    /// Mean age over pixels that have a commit. None when none have.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn mean_age(self) -> Option<f64> {
        let committed = self.red.saturating_sub(self.never);
        if committed == 0 {
            None
        } else {
            Some(f64::from(self.sum_age) / f64::from(committed))
        }
    }
}

/// The `lightingage` line. `queue` is `frame` or `round`, as `lightingsc` uses.
pub fn line(queue: &str, frame: u32, hist: &AgeHist) -> String {
    format!(
        "lightingage {queue} frame {frame} red {} never {} age0 {} age1_3 {} age4_7 {} age8_15 {} age16 {} predates {} postdates {} sum_age {} nostamp {} noslot {} slanted {} feed_n {} feed_sum {} feed_pre {} feed_post {} feed_none {} patch_n {} patch_sum {} patch_pre {} patch_post {} patch_none {} probes {}",
        hist.red,
        hist.never,
        hist.age0,
        hist.age1_3,
        hist.age4_7,
        hist.age8_15,
        hist.age16,
        hist.predates,
        hist.postdates,
        hist.sum_age,
        hist.nostamp,
        hist.noslot,
        hist.slanted,
        hist.feed_n,
        hist.feed_sum,
        hist.feed_pre,
        hist.feed_post,
        hist.feed_none,
        hist.patch_n,
        hist.patch_sum,
        hist.patch_pre,
        hist.patch_post,
        hist.patch_none,
        hist.feed_probes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_before_the_lamp_stamp_predates_it() {
        let (age, stamp) = classify(190, 160, 168);
        assert_eq!(age, Some(30));
        assert_eq!(stamp, Stamp::Predates);
        assert_eq!(age_slot(30), DIAG_AGE16);
    }

    #[test]
    fn the_stamp_frame_itself_postdates() {
        let (age, stamp) = classify(180, 168, 168);
        assert_eq!(age, Some(12));
        assert_eq!(stamp, Stamp::Postdates);
        assert_eq!(age_slot(12), DIAG_AGE8_15);
    }

    #[test]
    fn never_committed_and_no_stamp_are_their_own_classes() {
        assert_eq!(classify(10, 0, 168), (None, Stamp::Never));
        assert_eq!(classify(10, 8, 0).1, Stamp::Unstamped);
        assert_eq!(age_slot(0), DIAG_AGE0);
        assert_eq!(age_slot(3), DIAG_AGE1_3);
        assert_eq!(age_slot(7), DIAG_AGE4_7);
    }

    #[test]
    fn the_line_reads_the_diag_words_the_screen_pass_adds() {
        let mut diag = [0u32; 64];
        diag[DIAG_RED] = 100;
        diag[DIAG_NEVER] = 10;
        diag[DIAG_AGE0] = 20;
        diag[DIAG_AGE1_3] = 30;
        diag[DIAG_PREDATES] = 40;
        diag[DIAG_POSTDATES] = 50;
        diag[DIAG_SUM_AGE] = 270;
        diag[DIAG_NOSLOT] = 4;
        diag[DIAG_SLANTED] = 3;
        diag[DIAG_FEED_N] = 7;
        diag[DIAG_FEED_SUM] = 14;
        diag[DIAG_FEED_PROBES] = 7;
        diag[DIAG_PATCH_NONE] = 3;
        let hist = AgeHist::from_diag(&diag);
        let mean = hist.mean_age().unwrap_or(-1.0);
        assert!((mean - 3.0).abs() < 1.0e-9);
        let text = line("frame", 192, &hist);
        assert!(text.starts_with("lightingage frame frame 192 red 100 "));
        assert!(text.contains("predates 40 postdates 50 sum_age 270"));
        assert!(text.contains("noslot 4 slanted 3"));
        assert!(text.ends_with("feed_n 7 feed_sum 14 feed_pre 0 feed_post 0 feed_none 0 patch_n 0 patch_sum 0 patch_pre 0 patch_post 0 patch_none 3 probes 7"));
    }
}
