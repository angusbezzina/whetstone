// Deliberate breaches for the Whetstone value-epic demo; never merged.
fn port(text: &str) -> u16 {
    let parsed: Option<u16> = text.parse().ok();
    parsed.unwrap()
}
