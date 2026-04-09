# Whetstone Rules

These rules are extracted from dependency documentation by Whetstone. Follow them when writing code.

## Do

### rust.expect-over-unwrap (should)

SHOULD use .expect("reason") instead of .unwrap() in application code. .expect() documents invariants and produces actionable panic messages.


**Good:**
```
let port = env::var("PORT").expect("PORT env var must be set");

```
```
// In tests, unwrap is acceptable
#[test]
fn test_parse() {
    let result = parse("valid").unwrap();
}

```

**Bad:**
```
let port = env::var("PORT").unwrap();

```

Source: https://doc.rust-lang.org/book/ch09-03-to-panic-or-not-to-panic.html

### rust.timeout-on-http-clients (must)

MUST set explicit timeouts on HTTP clients. Most Rust HTTP libraries (reqwest, hyper, ureq) default to no timeout or very long timeouts.


**Good:**
```
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(30))
    .build()?;

```

**Bad:**
```
let client = reqwest::Client::new();

```

Source: https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout

### rust.error-context (should)

SHOULD add context to errors using .context() or .with_context() instead of .map_err(). Preserves the error chain and produces better diagnostics.


**Good:**
```
let data = fs::read_to_string(path)
    .context("failed to read config")?;

```

**Bad:**
```
let data = fs::read_to_string(path)
    .map_err(|e| anyhow!("read error: {}", e))?;

```

Source: https://docs.rs/anyhow/latest/anyhow/trait.Context.html

### rust.prefer-str-params (may)

MAY accept &str instead of String in function parameters when the function doesn't need ownership. Avoids unnecessary allocations at call sites.


**Good:**
```
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

```

**Bad:**
```
fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

```

Source: https://doc.rust-lang.org/book/ch04-03-slices.html

### rust.must-use-results (should)

SHOULD not discard Result values. Every Result should be handled via ?, .unwrap(), .expect(), match, or explicit let _ = assignment.


**Good:**
```
fs::remove_file(path)?;

```
```
let _ = fs::remove_file(path);

```

**Bad:**
```
fs::remove_file(path);

```

Source: https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute

### anyhow.context-over-map-err (should)

SHOULD use .context() or .with_context() instead of .map_err(|e| anyhow!(...)) to add context to errors. The context methods are more idiomatic, compose better with the ? operator, and preserve the original error chain.


**Good:**
```
let config = std::fs::read_to_string(path)
    .context("Failed to read config file")?;

```
```
let config = std::fs::read_to_string(path)
    .with_context(|| format!("Failed to read {}", path.display()))?;

```

**Bad:**
```
let config = std::fs::read_to_string(path)
    .map_err(|e| anyhow!("Failed to read config: {}", e))?;

```

Source: https://docs.rs/anyhow/latest/anyhow/trait.Context.html

### anyhow.expect-over-unwrap (should)

SHOULD use .expect("reason") instead of .unwrap() in application code using anyhow. When an operation is expected to succeed, .expect() documents the invariant and produces actionable panic messages.


**Good:**
```
let port = env::var("PORT").expect("PORT must be set");

```
```
let value = some_infallible_op().expect("internal: op is infallible");

```

**Bad:**
```
let port = env::var("PORT").unwrap();

```

Source: https://docs.rs/anyhow/latest/anyhow/

### reqwest.set-timeout (must)

MUST set an explicit timeout on reqwest clients. The default is no timeout, which means requests can hang indefinitely on unresponsive servers.


**Good:**
```
let client = reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(15))
    .build()?;

```
```
let client = reqwest::blocking::Client::builder()
    .timeout(Duration::from_secs(30))
    .connect_timeout(Duration::from_secs(5))
    .build()?;

```

**Bad:**
```
let client = reqwest::blocking::Client::new();

```
```
let client = reqwest::blocking::Client::builder()
    .user_agent("myapp/1.0")
    .build()?;

```

Source: https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout

### reqwest.check-status (should)

SHOULD call .error_for_status() or explicitly check status codes on reqwest responses. By default, reqwest treats 4xx/5xx as successful responses and returns the body without error.


**Good:**
```
let body = client.get(url)
    .send()?
    .error_for_status()?
    .text()?;

```
```
let resp = client.get(url).send()?;
if resp.status().is_success() {
    let body = resp.text()?;
}

```

**Bad:**
```
let body = client.get(url).send()?.text()?;

```

Source: https://docs.rs/reqwest/latest/reqwest/struct.Response.html#method.error_for_status

### clap.derive-over-builder (should)

SHOULD use the derive API (#[derive(Parser)]) instead of the builder API (Command::new()) for new CLI definitions. The derive API is more concise, type-safe, and the recommended approach in clap 4.x docs.


**Good:**
```
#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    verbose: bool,
}

```
```
#[derive(Parser)]
#[command(name = "myapp", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

```

**Bad:**
```
let matches = Command::new("myapp")
    .arg(Arg::new("verbose").short('v').long("verbose"))
    .get_matches();

```

Source: https://docs.rs/clap/latest/clap/_derive/index.html

## Don't

### serde_yaml.crate-deprecated (must)

serde_yaml 0.9 is officially deprecated and unmaintained. MUST migrate to an actively maintained alternative such as serde_yml, yaml-rust2, or marked_yaml.


**Avoid:**
```
[dependencies]
serde_yaml = "0.9"

```
```
use serde_yaml::from_str;

```

**Instead:**
```
[dependencies]
serde_yml = "0.0.12"

```
```
use serde_yml::from_str;

```

Source: https://crates.io/crates/serde_yaml


---

*Generated by [Whetstone](https://github.com/angusbezzina/whetstone) on 2026-04-09 from: anyhow, clap, reqwest, serde_yaml, whetstone:recommended/rust*
