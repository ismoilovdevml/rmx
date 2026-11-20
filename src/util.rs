pub fn bytes_to_readable(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];

    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::bytes_to_readable;

    #[test]
    fn units() {
        assert_eq!(bytes_to_readable(0), "0 B");
        assert_eq!(bytes_to_readable(1023), "1023 B");
        assert_eq!(bytes_to_readable(1024), "1.00 KB");
        assert_eq!(bytes_to_readable(5 * 1024 * 1024), "5.00 MB");
        assert_eq!(bytes_to_readable(3 << 30), "3.00 GB");
        assert_eq!(bytes_to_readable(2 << 40), "2.00 TB");
        assert_eq!(bytes_to_readable(4096 << 40), "4096.00 TB");
    }
}
