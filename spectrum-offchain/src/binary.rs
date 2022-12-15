use serde::Serialize;

pub fn prefixed_key<T: Serialize>(prefix: &str, id: &T) -> Vec<u8> {
    let mut key_bytes = bincode::serialize(prefix).unwrap();
    let id_bytes = bincode::serialize(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}
