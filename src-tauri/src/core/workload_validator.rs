use crate::core::certification_workload::CertificationWorkload;

#[derive(Debug, Clone)]
pub struct ValidationOutcome {
    pub valid: bool,
    pub failures: Vec<String>,
}

pub fn validate_output(
    workload: &CertificationWorkload,
    output: &str,
) -> ValidationOutcome {
    let mut failures = Vec::new();
    let trimmed = output.trim();
    let char_count = trimmed.chars().count();

    if char_count < workload.validation.minimum_output_chars {
        failures.push(format!(
            "output_too_short:{}<{}",
            char_count,
            workload.validation.minimum_output_chars
        ));
    }

    if char_count > workload.validation.maximum_output_chars {
        failures.push(format!(
            "output_too_long:{}>{}",
            char_count,
            workload.validation.maximum_output_chars
        ));
    }

    let lower = trimmed.to_lowercase();

    for term in &workload.validation.required_terms {
        if !lower.contains(&term.to_lowercase()) {
            failures.push(format!("missing_required_term:{term}"));
        }
    }

    if !workload.validation.required_json_keys.is_empty() {
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(serde_json::Value::Object(object)) => {
                for key in &workload.validation.required_json_keys {
                    if !object.contains_key(key) {
                        failures.push(format!("missing_json_key:{key}"));
                    }
                }

                for (key, expected) in &workload.validation.expected_json_values {
                    match object.get(key) {
                        Some(actual) if actual == expected => {}
                        Some(actual) => failures.push(format!(
                            "json_value_mismatch:{key}:expected={expected}:actual={actual}"
                        )),
                        None => failures.push(format!(
                            "missing_expected_json_value:{key}"
                        )),
                    }
                }
            }
            Ok(_) => failures.push("json_root_not_object".into()),
            Err(error) => failures.push(format!("invalid_json:{error}")),
        }
    }

    ValidationOutcome {
        valid: failures.is_empty(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::certification_workload::built_in_3b_realworld_v1;

    #[test]
    fn validator_rejects_missing_required_terms() {
        let pack = built_in_3b_realworld_v1().unwrap();
        let workload = &pack.workloads[0];

        let result = validate_output(
            workload,
            "This generic response deliberately omits the expected facts."
        );

        assert!(!result.valid);
        assert!(!result.failures.is_empty());
    }
}
