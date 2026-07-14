//! Search-engine and headless-browser runtime management.

use std::path::Path;

use a3s_search::browser_management::{
    browser_status, browser_statuses, install_browser, repair_browser, update_browser,
    BrowserRuntimeStatus, ManagedBrowser,
};

const HTTP_ENGINES: &[(&str, &str)] = &[
    ("ddg", "DuckDuckGo"),
    ("brave", "Brave Search"),
    ("wiki", "Wikipedia"),
    ("sogou", "Sogou"),
    ("360 / so360", "360 Search"),
];
const HEADLESS_ENGINES: &[(&str, &str, ManagedBrowser)] = &[
    ("google / g", "Google", ManagedBrowser::Chrome),
    ("baidu", "Baidu", ManagedBrowser::Chrome),
    ("bing_cn", "Bing China", ManagedBrowser::Chrome),
];

pub(crate) fn usage_text() -> &'static str {
    "usage:\n\
       a3s search status\n\
       a3s search engines\n\
       a3s search doctor\n\
       a3s search browser list\n\
       a3s search browser install <chrome|lightpanda>\n\
       a3s search browser update <chrome|lightpanda>\n\
       a3s search browser repair <chrome|lightpanda>\n"
}

pub(crate) async fn run(args: Vec<String>) -> anyhow::Result<()> {
    match args.as_slice() {
        [] => {
            print!("{}", usage_text());
            Ok(())
        }
        [arg] if matches!(arg.as_str(), "-h" | "--help" | "help") => {
            print!("{}", usage_text());
            Ok(())
        }
        [command] if command == "status" => {
            print_statuses();
            Ok(())
        }
        [command] if command == "engines" => {
            print_engines();
            Ok(())
        }
        [command] if command == "doctor" => doctor(),
        [family, command] if family == "browser" && command == "list" => {
            print_statuses();
            Ok(())
        }
        [family, action, browser] if family == "browser" => {
            let browser = parse_browser(browser)?;
            let status = match action.as_str() {
                "install" => install_browser(browser).await?,
                "update" => update_browser(browser).await?,
                "repair" => repair_browser(browser).await?,
                _ => return Err(usage_error(format!("unknown browser action '{action}'"))),
            };
            println!("✓ {} is ready", browser.as_str());
            print_browser_status(&status);
            Ok(())
        }
        _ => Err(usage_error("invalid search command")),
    }
}

fn usage_error(message: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!("{message}\n\n{}", usage_text())
}

fn parse_browser(value: &str) -> anyhow::Result<ManagedBrowser> {
    match value {
        "chrome" | "chromium" => Ok(ManagedBrowser::Chrome),
        "lightpanda" => Ok(ManagedBrowser::Lightpanda),
        _ => Err(usage_error(format!(
            "unknown browser '{value}'; choose chrome or lightpanda"
        ))),
    }
}

fn print_engines() {
    println!("HTTP engines (no browser required)");
    for (id, name) in HTTP_ENGINES {
        println!("  {id:<14} {name}");
    }
    println!("\nHeadless engines");
    for (id, name, browser) in HEADLESS_ENGINES {
        println!("  {id:<14} {name} ({})", browser.as_str());
    }
    println!("\nBrowser backends");
    println!("  chrome         Chrome/Chromium via CDP");
    println!("  lightpanda     Lightpanda via CDP (select with config.acl headless.backend)");
}

fn print_statuses() {
    println!("headless browser runtimes");
    for status in browser_statuses() {
        print_browser_status(&status);
    }
}

pub(crate) fn print_browser_status(status: &BrowserRuntimeStatus) {
    println!("  {}", status.browser.as_str());
    println!(
        "    available: {}",
        if status.available { "yes" } else { "no" }
    );
    println!("    source:    {:?}", status.source);
    println!(
        "    version:   {}",
        status.version.as_deref().unwrap_or("-")
    );
    println!(
        "    path:      {}",
        status
            .path
            .as_deref()
            .map(Path::display)
            .map(|path| path.to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    println!("    health:    {}", status.detail);
}

fn doctor() -> anyhow::Result<()> {
    print_statuses();
    let config_path = crate::config::find_config();
    let Some(config_path) = config_path else {
        println!("\nconfig: not found");
        println!(
            "result: search HTTP engines remain available; no configured headless engine to check"
        );
        return Ok(());
    };
    let config = a3s_code_core::config::CodeConfig::from_file(Path::new(&config_path))
        .map_err(|error| anyhow::anyhow!("failed to parse {config_path}: {error}"))?;
    println!("\nconfig: {config_path}");
    let Some(search) = config.search else {
        println!("result: no search block; web_search uses its HTTP defaults");
        return Ok(());
    };
    let requested = search
        .engines
        .iter()
        .filter(|(_, config)| config.enabled)
        .filter(|(name, _)| matches!(name.as_str(), "g" | "google" | "baidu" | "bing_cn"))
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    if requested.is_empty() {
        println!("result: no enabled headless engines; HTTP search is ready");
        return Ok(());
    }
    let backend = search
        .headless
        .as_ref()
        .map(|headless| headless.backend)
        .unwrap_or_default();
    let browser = if backend.is_lightpanda() {
        ManagedBrowser::Lightpanda
    } else {
        ManagedBrowser::Chrome
    };
    let status = browser_status(browser);
    println!("enabled headless engines: {}", requested.join(", "));
    println!("selected browser backend: {}", browser.as_str());
    if search.headless.is_none() {
        return Err(anyhow::anyhow!(
            "headless engines are enabled but search.headless is missing; add it to config.acl or use HTTP engines"
        ));
    }
    if !status.available {
        return Err(anyhow::anyhow!(
            "{} is unavailable; run `a3s search browser install {}`",
            browser.as_str(),
            browser.as_str()
        ));
    }
    println!("result: ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_parser_accepts_only_supported_backends() {
        assert_eq!(parse_browser("chrome").unwrap(), ManagedBrowser::Chrome);
        assert_eq!(
            parse_browser("lightpanda").unwrap(),
            ManagedBrowser::Lightpanda
        );
        assert!(parse_browser("firefox").is_err());
    }

    #[test]
    fn usage_lists_the_complete_browser_lifecycle() {
        let usage = usage_text();
        for action in ["list", "install", "update", "repair"] {
            assert!(usage.contains(action));
        }
    }
}
