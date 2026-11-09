use serde::Serializer;

pub fn serialize_optional_timestamp<S: Serializer>(
    ts: &Option<::prost_types::Timestamp>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match ts {
        Some(ts) => {
            let datetime = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
            match datetime {
                Some(dt) => serializer.serialize_str(&dt.to_rfc3339()),
                None => serializer.serialize_str(&format!("{}.{:09}", ts.seconds, ts.nanos)),
            }
        }
        None => serializer.serialize_none(),
    }
}

pub fn serialize_bytes_as_base64<S: Serializer>(
    bytes: &[u8],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if bytes.is_empty() {
        serializer.serialize_str("")
    } else {
        use base64::Engine;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }
}
