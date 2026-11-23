//! Prints what the pickers would offer on this computer: the installed
//! applications (with their stable ids), the browsers and the default one,
//! and whether an icon is found. Nothing is started.
//!
//!     cargo run -p deckpilot-apps --example list-apps

fn main() {
    let apps = deckpilot_apps::discover();
    let xdg = deckpilot_apps::xdg::XdgDirs::from_env();
    let with_icon = apps
        .iter()
        .filter(|a| deckpilot_apps::icons::icon(a, &xdg).is_some())
        .count();
    println!("{} applications, {with_icon} with an icon", apps.len());
    for app in &apps {
        println!("  {:<40} {:?} {}", app.name, app.kind, app.id);
    }
    let browsers = deckpilot_apps::browsers::browsers();
    println!("{} browsers:", browsers.browsers.len());
    for browser in &browsers.browsers {
        let default = if browsers.default.as_deref() == Some(browser.id.as_str()) {
            " (default)"
        } else {
            ""
        };
        println!("  {}{default}  {}", browser.name, browser.id);
    }
}
