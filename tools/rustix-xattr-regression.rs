//! Manual regression for RUSTSEC-2026-0486 and its caller compatibility.
//!
//! Run in a temporary Cargo project with this file as a [[test]] target.
//! Dependencies: rustix = { version = "=1.1.6", features = ["fs"] },
//! xattr = "=1.7.0", tempfile = "=3.27.0".
//! The filesystem must support extended attributes. Uses synthetic data only.
//! rustix 1.1.4 fails the empty-buffer test. With rustix 1.1.6,
//! xattr 1.6.1 fails the large-attribute test; xattr 1.7.0 passes.

#![cfg(any(target_os = "linux", target_os = "macos"))]

#[cfg(target_os = "linux")]
const ATTRIBUTE: &str = "user.pylon-regression";
#[cfg(target_os = "macos")]
const ATTRIBUTE: &str = "com.pylonsync.regression";

#[test]
fn empty_attribute_buffer_does_not_panic() {
    let file = tempfile::NamedTempFile::new().unwrap();
    xattr::set(file.path(), ATTRIBUTE, b"synthetic value").unwrap();
    // An empty MaybeUninit slice triggers the old panic without invoking
    // the Vec length corruption described by the advisory.
    let mut buffer: [std::mem::MaybeUninit<u8>; 0] = [];
    let (initialized, remaining) =
        rustix::fs::getxattr(file.path(), ATTRIBUTE, &mut buffer[..]).unwrap();
    assert!(initialized.is_empty());
    assert!(remaining.is_empty());
}

#[test]
fn large_attribute_survives_the_size_query() {
    let file = tempfile::NamedTempFile::new().unwrap();
    // Exceeds xattr's initial 4096-byte buffer and exercises its query API.
    let expected = vec![b'x'; 8192];
    xattr::set(file.path(), ATTRIBUTE, &expected).unwrap();
    let actual = xattr::get(file.path(), ATTRIBUTE).unwrap().unwrap();
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual, expected);
}
