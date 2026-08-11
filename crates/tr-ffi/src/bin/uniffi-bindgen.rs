//! The UniFFI bindings generator, vendored as a bin target of this crate.
//!
//! Building it here (rather than installing `uniffi-bindgen` globally) pins the
//! generator to the exact `uniffi` version that produced the scaffolding in
//! `lib.rs`. A mismatch between the two is the single most common cause of
//! "checksum mismatch" panics at Swift call time, and this makes it impossible.
fn main() {
    uniffi::uniffi_bindgen_main()
}
