use std::path::PathBuf;
use std::{env, fs};

use jsonschema::{Draft, Validator};
use serde_json::Value;

pub struct Case {
    pub name: &'static str,
    pub instance: Value,
    pub valid: bool,
}

fn schema(name: &str) -> Validator {
    let root = env::var_os("PANGRAM_CONTRACT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts"));
    let path = root.join(name);
    let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .build(&value)
        .unwrap()
}

pub fn assert_cases(schema_name: &str, cases: Vec<Case>) {
    let validator = schema(schema_name);

    for case in cases {
        let valid = validator.is_valid(&case.instance);
        assert_eq!(
            valid, case.valid,
            "{}: {} expected valid={}, got valid={}",
            schema_name, case.name, case.valid, valid
        );
    }
}
