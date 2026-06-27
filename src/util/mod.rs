pub mod atomic;

pub fn format_timestamp() -> String {
    format!(
        "{:016}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}
