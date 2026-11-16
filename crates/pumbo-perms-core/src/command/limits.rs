//! Staff limits of `/pp`: which players, groups, tracks and arguments a sender may change.

/// Proof that the limits were checked; only this module makes one.
pub(super) struct Checked(());

/// A change without limits.
pub(super) fn unlimited() -> Checked {
    Checked(())
}
