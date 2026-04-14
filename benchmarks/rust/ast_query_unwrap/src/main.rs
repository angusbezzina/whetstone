fn read_port(raw: &str) -> u16 {
    raw.parse().unwrap()
}

fn safer(raw: &str) -> u16 {
    raw.parse().expect("port must be valid")
}

fn chained(raw: Option<&str>) -> &str {
    raw
        .map(str::trim)
        .unwrap()
}
