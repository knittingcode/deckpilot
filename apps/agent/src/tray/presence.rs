//! Whether a `StatusNotifier` host shows the tray icon (Linux, DP-TRAY-01).
//!
//! On Linux, `tray-icon` hands the icon to libayatana-appindicator3, which
//! publishes it as a `StatusNotifierItem` on the session bus and registers it
//! with `org.kde.StatusNotifierWatcher`. Creating the icon succeeds whether
//! or not anything shows it: GNOME without an `AppIndicator` extension has no
//! watcher, and the icon (with its "Quit DeckPilot") is then invisible. The
//! agent therefore asks the bus, read-only: whether the watcher's name has
//! an owner, the watcher's `IsStatusNotifierHostRegistered` and
//! `RegisteredStatusNotifierItems` properties, and the process id behind
//! each registered item's bus name, to find its own icon among them.
//!
//! The answer is checked again now and then (a shell may start after the
//! agent at login, or an extension may be switched on or off): every
//! second at first, then less often (see [`poll_interval`]).
//!
//! The decision ([`check`]) works on [`StatusNotifierBus`], which tests
//! fake; [`SessionBus`] is the real session bus (zbus, which the agent
//! already links for the screen lock and the credential store).

use std::time::Duration;

use deckpilot_ipc::TrayState;
use tracing::debug;

/// The watcher's well-known name, and its interface.
#[cfg(target_os = "linux")]
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
/// The watcher's object path.
#[cfg(target_os = "linux")]
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

/// What [`check`] needs from the session bus.
pub(crate) trait StatusNotifierBus {
    /// Whether a watcher owns `org.kde.StatusNotifierWatcher`.
    fn watcher_present(&mut self) -> Result<bool, String>;
    /// The watcher's `IsStatusNotifierHostRegistered`.
    fn host_registered(&mut self) -> Result<bool, String>;
    /// The watcher's `RegisteredStatusNotifierItems`: a bus name, or a bus
    /// name followed by the item's object path (`:1.42/org/ayatana/…`).
    fn registered_items(&mut self) -> Result<Vec<String>, String>;
    /// The process id of the connection that owns `bus_name`.
    fn owner_pid(&mut self, bus_name: &str) -> Result<u32, String>;
}

/// Whether a `StatusNotifier` host shows the icon of process `pid`.
///
/// The agent's icon is told by the process id behind its bus name. In a
/// sandbox with a process id namespace of its own (Flatpak) the two never
/// match, so the answer there is never shown, only no host or unknown.
///
/// - no watcher, or a watcher without a host: [`TrayState::NoHost`];
/// - a host that lists an item of `pid`: [`TrayState::Shown`];
/// - otherwise (the bus failed, or the host does not list the icon yet):
///   [`TrayState::HostUnknown`].
#[cfg(test)]
pub(crate) fn check(bus: &mut dyn StatusNotifierBus, pid: u32) -> TrayState {
    match check_inner(bus, pid) {
        Ok(state) => state,
        Err(error) => {
            debug!(%error, "cannot tell whether a StatusNotifier host shows the tray icon");
            TrayState::HostUnknown
        }
    }
}

fn check_inner(bus: &mut dyn StatusNotifierBus, pid: u32) -> Result<TrayState, String> {
    if !bus.watcher_present()? || !bus.host_registered()? {
        return Ok(TrayState::NoHost);
    }
    for item in bus.registered_items()? {
        let Some(name) = item_bus_name(&item) else {
            continue;
        };
        // An item whose owner is gone or cannot be asked is someone else's
        // as far as this check goes.
        if bus.owner_pid(name).ok() == Some(pid) {
            return Ok(TrayState::Shown);
        }
    }
    Ok(TrayState::HostUnknown)
}

/// The bus name of a registered item (`:1.42/org/ayatana/…` → `:1.42`,
/// `org.kde.StatusNotifierItem-7-1` as it is), or `None` for an item
/// listed by object path only, whose owner cannot be told.
fn item_bus_name(item: &str) -> Option<&str> {
    let name = item.split_once('/').map_or(item, |(name, _)| name);
    (!name.is_empty()).then_some(name)
}

/// How long to wait before checking again, `elapsed` after the tray was
/// created: every second for 10 s (the icon registers within moments of
/// being created), every 3 s until a minute (a shell still starting at
/// login), then every 15 s (an extension switched on or off later).
pub(crate) fn poll_interval(elapsed: Duration) -> Duration {
    if elapsed < Duration::from_secs(10) {
        Duration::from_secs(1)
    } else if elapsed < Duration::from_secs(60) {
        Duration::from_secs(3)
    } else {
        Duration::from_secs(15)
    }
}

/// The session bus, through zbus.
#[cfg(target_os = "linux")]
pub(crate) struct SessionBus {
    connection: zbus::blocking::Connection,
}

#[cfg(target_os = "linux")]
impl SessionBus {
    pub(crate) fn connect() -> Result<Self, String> {
        zbus::blocking::Connection::session()
            .map(|connection| Self { connection })
            .map_err(|e| e.to_string())
    }

    fn dbus(&self) -> Result<zbus::blocking::fdo::DBusProxy<'_>, String> {
        zbus::blocking::fdo::DBusProxy::new(&self.connection).map_err(|e| e.to_string())
    }

    fn watcher(&self) -> Result<zbus::blocking::Proxy<'_>, String> {
        zbus::blocking::proxy::Builder::new(&self.connection)
            .destination(WATCHER)
            .and_then(|b| b.path(WATCHER_PATH))
            .and_then(|b| b.interface(WATCHER))
            .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
            .and_then(zbus::blocking::proxy::Builder::build)
            .map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "linux")]
impl StatusNotifierBus for SessionBus {
    fn watcher_present(&mut self) -> Result<bool, String> {
        let name = zbus::names::BusName::try_from(WATCHER).map_err(|e| e.to_string())?;
        self.dbus()?.name_has_owner(name).map_err(|e| e.to_string())
    }

    fn host_registered(&mut self) -> Result<bool, String> {
        self.watcher()?
            .get_property::<bool>("IsStatusNotifierHostRegistered")
            .map_err(|e| e.to_string())
    }

    fn registered_items(&mut self) -> Result<Vec<String>, String> {
        self.watcher()?
            .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
            .map_err(|e| e.to_string())
    }

    fn owner_pid(&mut self, bus_name: &str) -> Result<u32, String> {
        let name = zbus::names::BusName::try_from(bus_name).map_err(|e| e.to_string())?;
        self.dbus()?
            .get_connection_unix_process_id(name)
            .map_err(|e| e.to_string())
    }
}

/// Checks on a thread of its own, now and then ([`poll_interval`]), and
/// calls `report` with each new answer, starting with the first. Ends when
/// `report` returns `false` (the event loop is gone).
#[cfg(target_os = "linux")]
pub(crate) fn watch(report: impl Fn(TrayState) -> bool + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("tray-presence".into())
        .spawn(move || {
            let pid = std::process::id();
            let started = std::time::Instant::now();
            let mut bus: Option<SessionBus> = None;
            let mut last = None;
            loop {
                if bus.is_none() {
                    match SessionBus::connect() {
                        Ok(connected) => bus = Some(connected),
                        Err(error) => debug!(%error, "no session bus for the tray check"),
                    }
                }
                let checked = bus.as_mut().map(|bus| check_inner(bus, pid));
                let state = match checked {
                    Some(Ok(state)) => state,
                    Some(Err(error)) => {
                        debug!(%error, "cannot tell whether a StatusNotifier host shows the tray icon");
                        // Connected again for the next check, in case the
                        // connection itself is what failed.
                        bus = None;
                        TrayState::HostUnknown
                    }
                    None => TrayState::HostUnknown,
                };
                if last != Some(state) {
                    if !report(state) {
                        return;
                    }
                    last = Some(state);
                }
                std::thread::sleep(poll_interval(started.elapsed()));
            }
        });
    // The status then stays "host unknown", as the tray reported first.
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot check whether the tray icon is shown");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A session bus as a test describes it. `None` for a call that fails.
    #[derive(Default)]
    struct FakeBus {
        watcher: Option<bool>,
        host: Option<bool>,
        items: Option<Vec<String>>,
        pids: BTreeMap<String, u32>,
        asked_pids: Vec<String>,
    }

    impl StatusNotifierBus for FakeBus {
        fn watcher_present(&mut self) -> Result<bool, String> {
            self.watcher.ok_or_else(|| "no bus".into())
        }
        fn host_registered(&mut self) -> Result<bool, String> {
            self.host.ok_or_else(|| "no property".into())
        }
        fn registered_items(&mut self) -> Result<Vec<String>, String> {
            self.items.clone().ok_or_else(|| "no property".into())
        }
        fn owner_pid(&mut self, bus_name: &str) -> Result<u32, String> {
            self.asked_pids.push(bus_name.to_owned());
            self.pids
                .get(bus_name)
                .copied()
                .ok_or_else(|| "NameHasNoOwner".into())
        }
    }

    const PID: u32 = 4242;

    fn host_with(items: &[&str], pids: &[(&str, u32)]) -> FakeBus {
        FakeBus {
            watcher: Some(true),
            host: Some(true),
            items: Some(items.iter().map(|s| (*s).to_owned()).collect()),
            pids: pids.iter().map(|(n, p)| ((*n).to_owned(), *p)).collect(),
            ..FakeBus::default()
        }
    }

    #[test]
    fn no_watcher_means_no_host() {
        // GNOME without an AppIndicator extension: nobody owns the name.
        let mut bus = FakeBus {
            watcher: Some(false),
            ..FakeBus::default()
        };
        assert_eq!(check(&mut bus, PID), TrayState::NoHost);
        // A watcher without a host (a watcher left running, no panel).
        let mut bus = FakeBus {
            watcher: Some(true),
            host: Some(false),
            ..FakeBus::default()
        };
        assert_eq!(check(&mut bus, PID), TrayState::NoHost);
    }

    #[test]
    fn the_agents_own_item_means_shown() {
        // As KDE's and the GNOME extension's watchers list an
        // appindicator item: its unique name and object path.
        let mut bus = host_with(
            &[
                ":1.7/org/ayatana/NotificationItem/other",
                ":1.42/org/ayatana/NotificationItem/tray_icon_tray_app",
            ],
            &[(":1.7", 1), (":1.42", PID)],
        );
        assert_eq!(check(&mut bus, PID), TrayState::Shown);
        // A well-known item name, as other SNI libraries register.
        let mut bus = host_with(
            &["org.kde.StatusNotifierItem-4242-1"],
            &[("org.kde.StatusNotifierItem-4242-1", PID)],
        );
        assert_eq!(check(&mut bus, PID), TrayState::Shown);
    }

    #[test]
    fn a_host_that_does_not_list_the_icon_is_unknown() {
        // Not registered yet, or someone else's icons only.
        let mut bus = host_with(&[], &[]);
        assert_eq!(check(&mut bus, PID), TrayState::HostUnknown);
        let mut bus = host_with(
            &[
                ":1.7/StatusNotifierItem",
                "/org/ayatana/NotificationItem/x",
                ":1.9",
            ],
            &[(":1.7", 1)],
        );
        assert_eq!(check(&mut bus, PID), TrayState::HostUnknown);
        // An item by path only is not asked about; one whose owner is gone
        // (`:1.9`) is skipped.
        assert_eq!(bus.asked_pids, [":1.7", ":1.9"]);
    }

    #[test]
    fn a_failing_bus_is_unknown_not_missing() {
        // No session bus at all: the icon may well be shown.
        let mut bus = FakeBus::default();
        assert_eq!(check(&mut bus, PID), TrayState::HostUnknown);
        let mut bus = FakeBus {
            watcher: Some(true),
            ..FakeBus::default()
        };
        assert_eq!(check(&mut bus, PID), TrayState::HostUnknown);
        let mut bus = FakeBus {
            watcher: Some(true),
            host: Some(true),
            ..FakeBus::default()
        };
        assert_eq!(check(&mut bus, PID), TrayState::HostUnknown);
    }

    #[test]
    fn item_names() {
        assert_eq!(item_bus_name(":1.42/org/ayatana/x"), Some(":1.42"));
        assert_eq!(item_bus_name(":1.42"), Some(":1.42"));
        assert_eq!(
            item_bus_name("org.kde.StatusNotifierItem-1-1"),
            Some("org.kde.StatusNotifierItem-1-1")
        );
        assert_eq!(item_bus_name("/org/ayatana/x"), None);
        assert_eq!(item_bus_name(""), None);
    }

    #[test]
    fn checks_get_rarer() {
        let s = Duration::from_secs;
        assert_eq!(poll_interval(s(0)), s(1));
        assert_eq!(poll_interval(s(9)), s(1));
        assert_eq!(poll_interval(s(10)), s(3));
        assert_eq!(poll_interval(s(59)), s(3));
        assert_eq!(poll_interval(s(60)), s(15));
        assert_eq!(poll_interval(s(86_400)), s(15));
    }
}
