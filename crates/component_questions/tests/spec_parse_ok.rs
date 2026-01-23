use std::fs;
use std::path::PathBuf;

use questions::spec::SetupSpec;

#[test]
fn parse_spec_fixture() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("setup")
        .join("webex.yaml");
    let contents = fs::read_to_string(path).expect("read fixture");
    let spec: SetupSpec = serde_yaml_bw::from_str(&contents).expect("parse spec");
    assert_eq!(spec.provider_id, "webex");
    assert_eq!(spec.questions.len(), 2);
}
