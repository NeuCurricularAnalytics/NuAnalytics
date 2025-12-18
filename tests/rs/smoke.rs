//! Integration smoke tests for `nu_analytics`

use nu_analytics::get_version;

#[test]
fn version_is_not_empty() {
    let v = get_version();
    assert!(!v.trim().is_empty());
}
