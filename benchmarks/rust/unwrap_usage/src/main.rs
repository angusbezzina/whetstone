fn parse_port(raw: &str) -> u16 {
    raw.parse().unwrap()
}

fn safer_parse_port(raw: &str) -> u16 {
    raw.parse().expect("port must be a valid u16 because we validated it upstream")
}

fn config_value(raw: Option<&str>) -> &str {
    raw.unwrap()
}
