//! Where a control loop owns a resource, change the declaration, never the
//! resource (issue #3759).
//!
//! A reconciler compares live workers against a declared desired count and
//! trims the surplus on its own schedule. Acting on the resource directly
//! instead of the declaration fails in two ways, and the second one is the
//! dangerous one:
//!
//! - the change is *reverted* — silently, by a component doing its job, on
//!   its own schedule; the revert is not a bug, so it appears in no error
//!   log, and the operator's capacity fix is undone in minutes;
//! - the change treats a *recurring* cause as one-off — every worker the
//!   loop launches still carries the bad declared parameters, so the
//!   symptom (a prompt too long for a slot) returns as fast as it is
//!   cleared.
//!
//! The contract this module makes checkable:
//!
//! 1. **The declaration is the control loop's only capacity input.** There
//!    is no path in [`CapacityReconciler`] that submits or cancels a worker
//!    directly; the only way to change capacity is
//!    [`CapacityReconciler::set_declaration`], and
//!    [`CapacityReconciler::reconcile`] derives every action from the
//!    declaration plus the observed live state.
//! 2. **Declared parameters are reviewed against the workload's prompt
//!    ceiling before they take effect** (issue #3749). Because
//!    `--parallel` divides the model window, a slot serves
//!    `window / slots` tokens; below the fleet's smallest prompt the slot
//!    is unusable capacity, and zero `slots`/`hours` are rejected
//!    fail-closed. A declaration that fails the review never takes effect.
//! 3. **Every cancellation is a revert and is logged with its reason,
//!    naming the declaration being enforced** ([`RevertNotice::log_line`]),
//!    so an operator whose manual workers are trimmed learns what to
//!    change rather than chasing the revert as if it were a failure.
//! 4. **The two halves can run separately** (issue #4379). The reconciler
//!    is the only component that submits a missing worker, and it was
//!    disabled entirely (`--dry-run`) because its other half trims workers
//!    an operator holds warm on purpose — so the fleet could lose workers
//!    (rotated out by the wedged-worker watchdog) and never regain them.
//!    [`CapacityReconciler::with_no_trim`] splits the halves: the safe one
//!    (submit the deficit) runs live, the destructive one (trim the
//!    surplus) is off. A capability disabled for one of its behaviours is
//!    split, not switched off; and a remover (the rotation watchdog) must
//!    never be added to a fleet whose replacer is off, because every
//!    rotation then permanently reduces capacity.

use std::fmt;

/// Declared desired capacity for one model — the only input the control
/// loop reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityDeclaration {
    /// Model the declaration governs (e.g. `qwen3.8-27b`).
    pub model: String,
    /// Desired live worker count for the model.
    pub want: u32,
    /// Slurm partition workers are submitted to.
    pub part: String,
    /// Per-worker job hours. `0` is rejected: workers expire immediately,
    /// causing constant churn and expiry-driven endpoint staleness.
    pub hours: u32,
    /// `--parallel` slot count. Divides the model window, so
    /// `window / slots` is the tokens per slot a worker can serve.
    pub slots: u32,
}

/// A live worker as observed by the reconciler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveWorker {
    /// Scheduler job id; Slurm job ids are monotonic, so a larger id means
    /// the worker was submitted later.
    pub job_id: u64,
    /// Model the worker serves.
    pub model: String,
    /// Slurm partition the worker runs in.
    pub part: String,
    /// Job hours the worker was submitted with.
    pub hours: u32,
    /// `--parallel` slot count the worker was submitted with.
    pub slots: u32,
}

/// Why a declaration fails the prompt-ceiling review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationError {
    /// The declaration names no model, so the reconciler would own nothing.
    EmptyModel,
    /// The declaration names no partition, so submissions have nowhere to go.
    EmptyPartition,
    /// `slots` is `0`: `--parallel 0` would divide the window by nothing.
    ZeroSlots,
    /// `hours` is `0`: workers expire the moment they start, which is the
    /// constant-churn / endpoint-staleness failure mode.
    ZeroHours,
    /// `window / slots` is below the fleet's smallest prompt: every slot
    /// the loop submits is unusable capacity (issue #3749).
    TokensPerSlotBelowCeiling {
        /// The model's context window in tokens.
        window: u64,
        /// The declared `--parallel` slot count.
        slots: u32,
        /// `window / slots`: the tokens per slot the declaration produces.
        tokens_per_slot: u64,
        /// The fleet's smallest required prompt in tokens.
        prompt_ceiling: u64,
    },
}

impl fmt::Display for DeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyModel => {
                write!(f, "declaration names no model; refusing to take effect")
            }
            Self::EmptyPartition => {
                write!(f, "declaration names no partition; submissions would have nowhere to go")
            }
            Self::ZeroSlots => {
                write!(f, "slots is 0: --parallel 0 would divide the window by nothing")
            }
            Self::ZeroHours => write!(
                f,
                "hours is 0: workers would expire immediately (constant churn, endpoint staleness)"
            ),
            Self::TokensPerSlotBelowCeiling {
                window,
                slots,
                tokens_per_slot,
                prompt_ceiling,
            } => write!(
                f,
                "slots {slots} leaves {tokens_per_slot} tokens/slot ({window} / {slots}), below the workload prompt ceiling of {prompt_ceiling}; the declared capacity is unusable"
            ),
        }
    }
}

impl std::error::Error for DeclarationError {}

/// Review a declaration against the workload's prompt ceiling.
///
/// `window` is the model's context window in tokens and `prompt_ceiling` is
/// the fleet's smallest required prompt in tokens. Because `--parallel`
/// divides the window, a slot serves `window / slots` tokens; below the
/// ceiling no fleet prompt fits a slot, and the capacity the loop would
/// launch is unusable. This is the review the declared parameters were
/// missing in issue #3749 (262144 / 8 = 32768 tokens/slot against a fleet
/// whose smallest prompt did not fit).
pub fn validate_declaration(
    declaration: &CapacityDeclaration,
    window: u64,
    prompt_ceiling: u64,
) -> Result<(), DeclarationError> {
    if declaration.model.trim().is_empty() {
        return Err(DeclarationError::EmptyModel);
    }
    if declaration.part.trim().is_empty() {
        return Err(DeclarationError::EmptyPartition);
    }
    if declaration.slots == 0 {
        return Err(DeclarationError::ZeroSlots);
    }
    if declaration.hours == 0 {
        return Err(DeclarationError::ZeroHours);
    }
    let tokens_per_slot = window / u64::from(declaration.slots);
    if tokens_per_slot < prompt_ceiling {
        return Err(DeclarationError::TokensPerSlotBelowCeiling {
            window,
            slots: declaration.slots,
            tokens_per_slot,
            prompt_ceiling,
        });
    }
    Ok(())
}

/// A reconciliation cancellation of a worker the operator placed by hand.
///
/// Trimming the surplus is the reconciler doing its job, not a bug — so the
/// log line must say *why* and name the declaration being enforced, or the
/// operator chases the revert as if it were a failure and the real fix (the
/// declaration) is never found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertNotice {
    /// The job id being cancelled.
    pub job_id: u64,
    /// The declaration being enforced, in full, so the log names both what
    /// is being trimmed to and the parameters to change.
    pub declaration: CapacityDeclaration,
    /// Live workers observed for the model before this pass trimmed.
    pub live: u32,
}

impl RevertNotice {
    /// The log line the reconciler must emit for this cancellation.
    pub fn log_line(&self) -> String {
        format!(
            "revert: CANCEL job {} — enforcing declaration {} (want {}, part {}, hours {}, slots {}); {} live, trimming surplus. Capacity changes go through the declaration, never the resource.",
            self.job_id,
            self.declaration.model,
            self.declaration.want,
            self.declaration.part,
            self.declaration.hours,
            self.declaration.slots,
            self.live,
        )
    }
}

/// Outcome of one reconcile pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Workers to submit so the model's live count reaches the declared
    /// `want`; `0` when the live count already meets or exceeds it.
    pub submit: u32,
    /// The workers this pass trims, newest first, with the revert notice
    /// each cancellation must log.
    pub cancels: Vec<RevertNotice>,
}

/// The control loop's view of one model's capacity.
///
/// The declaration is its only capacity input: there is no method here that
/// submits or cancels a worker directly. Change capacity by changing the
/// declaration with [`CapacityReconciler::set_declaration`];
/// [`CapacityReconciler::reconcile`] then derives the actions that make
/// reality match it.
#[derive(Debug, Clone)]
pub struct CapacityReconciler {
    declaration: CapacityDeclaration,
    window: u64,
    prompt_ceiling: u64,
    /// `true` when the destructive half is split off: the reconciler
    /// submits the deficit and leaves any surplus alone (issue #4379).
    no_trim: bool,
}

impl CapacityReconciler {
    /// Build a reconciler, reviewing the declaration against the prompt
    /// ceiling first; a declaration that fails the review never takes
    /// effect.
    pub fn new(
        declaration: CapacityDeclaration,
        window: u64,
        prompt_ceiling: u64,
    ) -> Result<Self, DeclarationError> {
        validate_declaration(&declaration, window, prompt_ceiling)?;
        Ok(Self {
            declaration,
            window,
            prompt_ceiling,
            no_trim: false,
        })
    }

    /// Build a reconciler with the destructive half split off: it submits
    /// the deficit and never cancels a worker (issue #4379).
    ///
    /// This is the mode the cron entry runs in, and it exists because the
    /// whole reconciler was pinned to dry-run: trimming the surplus would
    /// cancel workers an operator holds warm on purpose, so the only
    /// component that can submit a missing worker never ran — and a
    /// remover (the wedged-worker rotation) with no live replacer drains
    /// the fleet one rotation at a time. The declaration review is
    /// identical; only the trim is off.
    pub fn with_no_trim(
        declaration: CapacityDeclaration,
        window: u64,
        prompt_ceiling: u64,
    ) -> Result<Self, DeclarationError> {
        let mut reconciler = Self::new(declaration, window, prompt_ceiling)?;
        reconciler.no_trim = true;
        Ok(reconciler)
    }

    /// Whether the destructive half is split off: the reconciler submits
    /// deficits and leaves any surplus alone.
    pub fn no_trim(&self) -> bool {
        self.no_trim
    }

    /// The declaration currently in force.
    pub fn declaration(&self) -> &CapacityDeclaration {
        &self.declaration
    }

    /// Change capacity by changing the declaration. A declaration that
    /// fails the prompt-ceiling review is rejected and the previous
    /// declaration stays in force (fail-closed): the loop keeps enforcing
    /// the last declaration that passed.
    pub fn set_declaration(
        &mut self,
        declaration: CapacityDeclaration,
    ) -> Result<(), DeclarationError> {
        validate_declaration(&declaration, self.window, self.prompt_ceiling)?;
        self.declaration = declaration;
        Ok(())
    }

    /// Compare live workers against the declaration and derive the actions
    /// that make reality match it.
    ///
    /// Workers of other models are not this declaration's resource and are
    /// left alone. For the declared model, the surplus over `want` is
    /// trimmed newest first (largest job id first — Slurm job ids are
    /// monotonic, which is how the reconciler in issue #3759 cancelled the
    /// operator's just-submitted replacements). Every cancellation carries
    /// a [`RevertNotice`] naming the declaration being enforced. In
    /// no-trim mode (built by [`CapacityReconciler::with_no_trim`]) the
    /// deficit is still submitted, but the surplus is never cancelled:
    /// the safe half runs without the destructive one (issue #4379).
    pub fn reconcile(&self, live: &[LiveWorker]) -> ReconcileReport {
        let ours: Vec<&LiveWorker> = live
            .iter()
            .filter(|worker| worker.model == self.declaration.model)
            .collect();
        let want = self.declaration.want as u64;
        if ours.len() as u64 <= want {
            return ReconcileReport {
                submit: (want - ours.len() as u64) as u32,
                cancels: Vec::new(),
            };
        }
        if self.no_trim {
            // Surplus: nothing to submit, and the surplus is left alone —
            // the destructive half is split off, so a worker an operator
            // holds warm is never cancelled by this pass (issue #4379).
            return ReconcileReport {
                submit: 0,
                cancels: Vec::new(),
            };
        }
        let surplus = ours.len() - want as usize;
        let live = ours.len() as u32;
        let mut ordered = ours;
        // Newest first; the stable sort keeps input order for equal job
        // ids, so the report is deterministic for a given observation.
        ordered.sort_by(|a, b| b.job_id.cmp(&a.job_id));
        let cancels = ordered
            .iter()
            .take(surplus)
            .map(|worker| RevertNotice {
                job_id: worker.job_id,
                declaration: self.declaration.clone(),
                live,
            })
            .collect();
        ReconcileReport { submit: 0, cancels }
    }
}
