//! Declared reads and writes. Conflicting jobs do not share a parallel batch.

use crate::Class;

/// Read or write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The job reads the key and does not write it.
    Read,
    /// The job writes the key.
    Write,
}

/// A component or resource a job touches.
///
/// Keys come from a name. [`AccessKey::of`] uses the Rust type name, which is
/// stable for one build. Two jobs conflict when they name the same key and at
/// least one writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccessKey(u64);

impl AccessKey {
    /// A key from a stable name, for example `"health"`.
    #[must_use]
    pub fn named(name: &str) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in name.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(hash)
    }

    /// A key for the type `T`.
    #[must_use]
    pub fn of<T: 'static>() -> Self {
        Self::named(std::any::type_name::<T>())
    }
}

/// One declared access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    /// What is touched.
    pub key: AccessKey,
    /// Read or write.
    pub mode: Mode,
}

/// True when the two sets cannot run at the same time.
#[must_use]
pub fn conflicts(left: &[Access], right: &[Access]) -> bool {
    left.iter().any(|one| {
        right.iter().any(|other| {
            one.key == other.key && (one.mode == Mode::Write || other.mode == Mode::Write)
        })
    })
}

/// Split jobs into conflict-free batches, in the order given.
///
/// A later batch starts when the next job conflicts with one already in the
/// current batch. Callers pass jobs in index order so a conflicting pair keeps
/// that order.
#[must_use]
pub fn conflict_batches(access: &[Vec<Access>]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    for (index, job_access) in access.iter().enumerate() {
        let fits = batches.last().is_some_and(|batch| {
            batch
                .iter()
                .all(|other| !conflicts(&access[*other], job_access))
        });
        if fits {
            if let Some(batch) = batches.last_mut() {
                batch.push(index);
            }
        } else {
            batches.push(vec![index]);
        }
    }
    batches
}

/// Why a parallel group was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// Two jobs in one parallel group write the same key, or one reads what the other writes.
    Conflict {
        /// Earlier job.
        left: usize,
        /// Later job.
        right: usize,
    },
    /// The same stage used an index twice.
    DuplicateIndex {
        /// Stage number.
        stage: u32,
        /// Repeated index.
        index: u32,
    },
    /// A stage skipped an index.
    Gap {
        /// Stage number.
        stage: u32,
    },
    /// A job called [`crate::Scheduler::run`] while it was already running.
    Rejected,
    /// A job panicked. The workers were joined.
    Panicked,
    /// The deque could not hold the batch.
    Full,
    /// A worker thread did not start.
    Spawn,
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { left, right } => {
                write!(formatter, "jobs {left} and {right} conflict")
            }
            Self::DuplicateIndex { stage, index } => {
                write!(formatter, "stage {stage} index {index} is repeated")
            }
            Self::Gap { stage } => write!(formatter, "stage {stage} skipped an index"),
            Self::Rejected => formatter.write_str("a job waited inside its own stage"),
            Self::Panicked => formatter.write_str("a job panicked"),
            Self::Full => formatter.write_str("the work queue is full"),
            Self::Spawn => formatter.write_str("a worker thread did not start"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Refuse the set when any pair conflicts.
///
/// # Errors
///
/// [`ScheduleError::Conflict`] for the first conflicting pair in order.
pub fn ensure_parallel(access: &[Vec<Access>]) -> Result<(), ScheduleError> {
    for left in 0..access.len() {
        for right in (left + 1)..access.len() {
            if conflicts(&access[left], &access[right]) {
                return Err(ScheduleError::Conflict { left, right });
            }
        }
    }
    Ok(())
}

/// Class order inside a stage: Must, then Soon, then Slack. Pumped is not run.
#[must_use]
pub fn class_rank(class: Class) -> u8 {
    match class {
        Class::Must => 0,
        Class::Soon => 1,
        Class::Slack => 2,
        Class::Pumped => 3,
    }
}
