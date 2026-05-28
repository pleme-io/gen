//! Exercise `#[derive(BackendError)]` — auto-emits the trait impl
//! from per-variant attributes.
//!
//! Round-trip with Discriminant: `kind()` delegates to `discriminant()`,
//! so the wire string is the same.

use gen_platform::{BackendError as BackendErrorDerive, Discriminant};

/// Caller-supplied trait — the derive emits `impl <this trait> for <enum>`.
/// In production, this lives in magma-converge; the test puts it inline
/// to verify the derive's `trait_path` defaulting works.
trait BackendError {
    fn is_retryable(&self) -> bool;
    fn is_auth_failure(&self) -> bool {
        false
    }
    fn kind(&self) -> &'static str;
}

#[derive(Debug, Discriminant, BackendErrorDerive)]
#[discriminant(method = "discriminant", case = "snake")]
enum MyError {
    NotFound,

    #[backend_error(auth)]
    PermissionDenied,

    #[backend_error(transient)]
    Transient,

    Permanent,
}

#[test]
fn is_retryable_true_only_for_transient() {
    assert!(!MyError::NotFound.is_retryable());
    assert!(!MyError::PermissionDenied.is_retryable());
    assert!(MyError::Transient.is_retryable());
    assert!(!MyError::Permanent.is_retryable());
}

#[test]
fn is_auth_failure_true_only_for_permission_denied() {
    assert!(!MyError::NotFound.is_auth_failure());
    assert!(MyError::PermissionDenied.is_auth_failure());
    assert!(!MyError::Transient.is_auth_failure());
    assert!(!MyError::Permanent.is_auth_failure());
}

#[test]
fn kind_delegates_to_discriminant() {
    assert_eq!(MyError::NotFound.kind(), MyError::NotFound.discriminant());
    assert_eq!(MyError::PermissionDenied.kind(), "permission_denied");
    assert_eq!(MyError::Transient.kind(), "transient");
    assert_eq!(MyError::Permanent.kind(), "permanent");
}

// ── Multi-variant tags ────────────────────────────────────────

#[derive(Debug, Discriminant, BackendErrorDerive)]
#[discriminant(method = "discriminant", case = "snake")]
enum WebhookProblem {
    #[backend_error(auth)]
    MissingHeader,

    #[backend_error(auth)]
    InvalidSignature,

    BadPayload,

    #[backend_error(transient)]
    OidcUnreachable,
}

#[test]
fn multiple_variants_can_share_tag() {
    assert!(WebhookProblem::MissingHeader.is_auth_failure());
    assert!(WebhookProblem::InvalidSignature.is_auth_failure());
    assert!(!WebhookProblem::BadPayload.is_auth_failure());
    assert!(!WebhookProblem::OidcUnreachable.is_auth_failure());
    assert!(WebhookProblem::OidcUnreachable.is_retryable());
}

// ── Empty case (all variants permanent) ───────────────────────

#[derive(Debug, Discriminant, BackendErrorDerive)]
#[discriminant(method = "discriminant", case = "snake")]
enum AllPermanent {
    BadConfig,
    MissingDep,
}

#[test]
fn no_transient_variants_means_no_retries() {
    assert!(!AllPermanent::BadConfig.is_retryable());
    assert!(!AllPermanent::MissingDep.is_retryable());
    assert!(!AllPermanent::BadConfig.is_auth_failure());
}
