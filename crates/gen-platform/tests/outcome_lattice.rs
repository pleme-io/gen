//! Exercise `#[derive(OutcomeLattice)]` — auto-emits the trait impl
//! from per-variant severity attributes.

use gen_platform::OutcomeLattice as OutcomeLatticeDerive;

/// Caller-supplied trait — the derive emits `impl <this trait> for <enum>`.
/// In production, this lives in magma-converge::outcome.
trait OutcomeLattice: Clone + PartialEq {
    fn severity(&self) -> u32;
    fn baseline() -> Self;
    fn worst(&self, other: &Self) -> Self {
        if self.severity() >= other.severity() {
            self.clone()
        } else {
            other.clone()
        }
    }
    fn best(&self, other: &Self) -> Self {
        if self.severity() <= other.severity() {
            self.clone()
        } else {
            other.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, OutcomeLatticeDerive)]
enum ReadyState {
    #[outcome(severity = 0, baseline)]
    Ready,
    #[outcome(severity = 1)]
    Unknown,
    #[outcome(severity = 2)]
    InProgress { reason: String },
    #[outcome(severity = 3)]
    Failed { reason: String },
}

#[test]
fn severity_per_variant() {
    assert_eq!(ReadyState::Ready.severity(), 0);
    assert_eq!(ReadyState::Unknown.severity(), 1);
    assert_eq!(ReadyState::InProgress { reason: "x".into() }.severity(), 2,);
    assert_eq!(ReadyState::Failed { reason: "y".into() }.severity(), 3,);
}

#[test]
fn baseline_returns_marked_variant() {
    assert_eq!(ReadyState::baseline(), ReadyState::Ready);
}

#[test]
fn worst_returns_higher_severity() {
    let a = ReadyState::Ready;
    let b = ReadyState::Failed { reason: "x".into() };
    assert!(matches!(a.worst(&b), ReadyState::Failed { .. }));
    assert!(matches!(b.worst(&a), ReadyState::Failed { .. }));
}

#[test]
fn best_returns_lower_severity() {
    let a = ReadyState::Ready;
    let b = ReadyState::Failed { reason: "x".into() };
    assert_eq!(a.best(&b), ReadyState::Ready);
}

#[derive(Clone, Debug, PartialEq, Eq, OutcomeLatticeDerive)]
enum ApplyStatus {
    #[outcome(severity = 0, baseline)]
    Empty,
    #[outcome(severity = 0)]
    AllSucceeded,
    #[outcome(severity = 1)]
    SucceededWithSkipped,
    #[outcome(severity = 2)]
    Failed,
    #[outcome(severity = 3)]
    Conflict,
}

#[test]
fn tied_severities_supported() {
    assert_eq!(ApplyStatus::Empty.severity(), 0);
    assert_eq!(ApplyStatus::AllSucceeded.severity(), 0);
    // Tie-break: self wins for determinism.
    assert_eq!(
        ApplyStatus::Empty.worst(&ApplyStatus::AllSucceeded),
        ApplyStatus::Empty,
    );
}
