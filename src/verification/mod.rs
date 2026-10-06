use crate::{
    contracts::ContractObservation,
    types::{Check, CheckStatus, RegistryMatch, Status},
};

pub fn verify(
    known: Option<&RegistryMatch>,
    observed: Option<&ContractObservation>,
) -> (Status, Vec<Check>) {
    let Some(known) = known else {
        return (Status::Unknown, vec![]);
    };
    let mut checks = vec![compare(
        "exists",
        "true".into(),
        observed.map(|v| v.exists.to_string()),
    )];
    if let Some(expected) = &known.expected_name {
        checks.push(compare(
            "name",
            expected.clone(),
            observed.and_then(|v| v.name.clone()),
        ));
    }
    if let Some(expected) = &known.expected_symbol {
        checks.push(compare(
            "symbol",
            expected.clone(),
            observed.and_then(|v| v.symbol.clone()),
        ));
    }
    if let Some(expected) = known.expected_decimals {
        checks.push(compare(
            "decimals",
            expected.to_string(),
            observed.and_then(|v| v.decimals.map(|n| n.to_string())),
        ));
    }
    if let Some(expected) = &known.expected_runtime_code_sha256 {
        checks.push(compare(
            "runtimeCodeSha256",
            expected.clone(),
            observed.map(|v| v.runtime_code_sha256.clone()),
        ));
    }
    if let Some(expected) = &known.expected_implementation {
        let actual = observed.and_then(|v| match &v.proxy {
            crate::proxy::ProxyObservation::Detected { implementation, .. } => {
                Some(implementation.clone())
            }
            _ => None,
        });
        checks.push(compare(
            "implementation",
            expected.to_ascii_lowercase(),
            actual,
        ));
    }
    if let Some(expected) = &known.expected_admin {
        checks.push(compare(
            "contractAdmin",
            expected.to_ascii_lowercase(),
            observed.and_then(|v| v.contract_admin.clone()),
        ));
    }
    let status = if checks.iter().any(|c| c.status == CheckStatus::Mismatched) {
        Status::Mismatch
    } else if checks.len() > 1 && checks.iter().all(|c| c.status == CheckStatus::Verified) {
        Status::Verified
    } else {
        Status::Partial
    };
    (status, checks)
}

fn compare(field: &str, expected: String, actual: Option<String>) -> Check {
    let status = match &actual {
        None => CheckStatus::Unavailable,
        Some(value) if value == &expected => CheckStatus::Verified,
        Some(_) => CheckStatus::Mismatched,
    };
    Check {
        field: field.into(),
        expected,
        actual,
        status,
    }
}
