pub fn view_hash(ops_json: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in ops_json.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_hash_deterministic() {
        let h1 = view_hash("hello");
        let h2 = view_hash("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn view_hash_distinct_inputs() {
        let h1 = view_hash("aaa");
        let h2 = view_hash("bbb");
        assert_ne!(h1, h2);
    }

    #[test]
    fn view_hash_length_is_16() {
        let h = view_hash("some input");
        assert_eq!(h.len(), 16);
    }

    #[test]
    fn view_hash_empty_string() {
        let h = view_hash("");
        assert_eq!(h.len(), 16);
    }
}
