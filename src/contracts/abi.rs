use sha3::{Digest, Keccak256};
pub fn selector(signature: &str) -> String {
    format!(
        "0x{}",
        hex::encode(&Keccak256::digest(signature.as_bytes())[..4])
    )
}
pub fn bytes(value: &str) -> Option<Vec<u8>> {
    hex::decode(value.strip_prefix("0x")?).ok()
}
pub fn word(value: &str) -> Option<[u8; 32]> {
    bytes(value)?.try_into().ok()
}
pub fn small_uint(value: &str) -> Option<u64> {
    let w = word(value)?;
    if w[..24].iter().any(|v| *v != 0) {
        return None;
    }
    Some(u64::from_be_bytes(w[24..].try_into().ok()?))
}
pub fn address(value: &str) -> Option<String> {
    let w = word(value)?;
    if w[..12].iter().any(|v| *v != 0) || w[12..].iter().all(|v| *v == 0) {
        return None;
    }
    Some(format!("0x{}", hex::encode(&w[12..])))
}
pub fn boolean(value: &str) -> Option<bool> {
    match small_uint(value)? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}
pub fn text(value: &str) -> Option<String> {
    let data = bytes(value)?;
    let raw = if data.len() == 32 {
        let end = data.iter().position(|v| *v == 0).unwrap_or(32);
        if data[end..].iter().any(|v| *v != 0) {
            return None;
        }
        &data[..end]
    } else {
        if data.len() < 64 || data.len() % 32 != 0 {
            return None;
        }
        if small_uint(&format!("0x{}", hex::encode(&data[..32])))? != 32 {
            return None;
        }
        let size =
            usize::try_from(small_uint(&format!("0x{}", hex::encode(&data[32..64])))?).ok()?;
        if size > 4096 {
            return None;
        }
        let end = 64usize.checked_add(size)?;
        let padded = end.checked_add(31)? / 32 * 32;
        if data.len() != padded || data.get(end..)?.iter().any(|v| *v != 0) {
            return None;
        }
        data.get(64..end)?
    };
    let result = std::str::from_utf8(raw).ok()?;
    if result.chars().any(char::is_control) {
        return None;
    }
    Some(result.into())
}
/// Convert full uint256 to decimal without floating point or truncation.
pub fn uint256(value: &str) -> Option<String> {
    let w = word(value)?;
    let mut digits = vec![0u16];
    for byte in w {
        let mut carry = u16::from(byte);
        for digit in &mut digits {
            let next = *digit * 256 + carry;
            *digit = next % 10;
            carry = next / 10;
        }
        while carry != 0 {
            digits.push(carry % 10);
            carry /= 10;
        }
    }
    Some(
        digits
            .iter()
            .rev()
            .map(|d| char::from(b'0' + *d as u8))
            .collect(),
    )
}
