//! The operating-system signals provider (`os-signals`) and its Microsoft
//! Teams profile: **detection only** (`docs/calls.md`, "Microsoft Teams:
//! detection only").
//!
//! Microsoft retired the Teams desktop client's device API on 2026-06-30;
//! no supported mechanism controls Teams. What the operating system shows
//! is which app holds an audio capture session, and whether a camera is in
//! use. From that this provider infers "a call or meeting is probably
//! active" and reports it with [`Evidence::OsSignal`] (the engine waits
//! 2 s before it believes it, and 8 s before it lets it go). It never
//! claims more:
//!
//! - **Every control is unsupported** (`providerHasNoApi`). A command is
//!   refused before anything happens.
//! - **The microphone's mute state is unknown**: Teams may hold the
//!   microphone while muted, so "holds the microphone" is never shown as
//!   "not muted".
//! - **The camera** is reported only where the platform attributes it to the
//!   app (`PipeWire` streams); a camera "in use" by an unknown app is not
//!   the call's.
//! - **Linux**: Teams there is a browser tab; a browser holding the
//!   microphone is not attributed to Teams.
//!
//! **What leaves this module**: only the call observation (no process
//! names, ids or command lines). The executable names of capture sessions
//! are compared against a compiled list ([`Profile::teams`]) and dropped.
//!
//! **No live queries here.** The platform sources (Core Audio and
//! `AVCapture` on macOS, WASAPI on Windows, `PipeWire` on Linux) produce
//! [`Signals`] through [`SignalSource`]; this crate has the parsers of
//! their outputs and a fixture source for the tests. Whether the macOS
//! queries need a TCC prompt is not known and is an owner verification
//! before any live source is built: a signal that would prompt is dropped,
//! never prompted for.
//!
//! **Identity is the executable name only, so it can be spoofed**: any
//! process named like Teams that holds the microphone shows a call (an
//! indicator only, no control exists). Before a live source ships, it must
//! pass a verified identity instead of a bare name: on macOS the bundle id
//! (`com.microsoft.teams2`) with its code-signing team id, on Windows the
//! package family (`MSTeams_8wekyb3d8bbwe`).

use serde::Deserialize;

use crate::clock::Instant;
use crate::provider::{
    AppVersion, Availability, CallCommand, CallKey, CallKind, CallObservation, CallProvider,
    Capability, CapabilitySet, CommandRefusal, CommandToken, DisconnectReason, Discovery, Evidence,
    ObservedPhase, ProviderCx, ProviderError, ProviderEvent, ProviderId, ProviderSession, Support,
    Tri, UnsupportedReason,
};
use crate::snapshot::FailureCode;

#[cfg(test)]
#[path = "os_signals/tests.rs"]
mod tests;

/// The platform the signals come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

/// One audio capture session: the executable name of the process that
/// holds it (the file name only, never a path or a command line). Its
/// `Debug` leaves the name out, so a sample logged by mistake names no app.
#[derive(Clone, PartialEq, Eq)]
pub struct Capture {
    pub executable: String,
    pub active: bool,
}

/// A camera stream attributed to an app (`PipeWire` only). Its `Debug`
/// leaves the name out.
#[derive(Clone, PartialEq, Eq)]
pub struct CameraStream {
    pub executable: String,
    pub active: bool,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CameraStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraStream")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

/// What the operating system shows at one instant.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signals {
    pub captures: Vec<Capture>,
    /// Camera streams with their app, where the platform attributes them.
    pub cameras: Vec<CameraStream>,
    /// Some camera is in use by some app (macOS: not attributed).
    pub camera_in_use: Tri,
}

/// Why a source gave no signals: a stable code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the signals source failed: {0}")]
pub struct SourceError(pub FailureCode);

/// Where the signals come from: a platform source, or fixtures.
pub trait SignalSource: Send {
    fn platform(&self) -> Platform;
    fn sample(&mut self, now: Instant) -> Result<Signals, SourceError>;
}

/// The apps a profile watches for: a compiled list per platform, compared
/// by executable name without case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: &'static str,
    mac_os: &'static [&'static str],
    windows: &'static [&'static str],
}

impl Profile {
    /// Microsoft Teams (the new client and the classic one). On Linux
    /// there is no Teams desktop client: nothing is attributed.
    pub fn teams() -> Self {
        Self {
            id: "teams",
            mac_os: &[
                "MSTeams",
                "Microsoft Teams",
                "Microsoft Teams (work or school)",
            ],
            windows: &["ms-teams.exe", "Teams.exe"],
        }
    }

    /// Whether `executable` is one of the profile's apps on `platform`.
    pub fn matches(&self, platform: Platform, executable: &str) -> bool {
        let names = match platform {
            Platform::MacOs => self.mac_os,
            Platform::Windows => self.windows,
            Platform::Linux => &[],
        };
        names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(executable))
    }

    /// Whether the profile can attribute anything on `platform`.
    pub fn attributes_on(&self, platform: Platform) -> bool {
        !matches!(platform, Platform::Linux)
    }
}

/// What the signals say about the profile's app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribution {
    /// The app holds an active audio capture session.
    pub holds_microphone: bool,
    /// The app's own camera stream is active (only where attributed).
    pub camera: Tri,
}

/// Attributes `signals` to `profile`'s app on `platform`.
pub fn attribute(profile: &Profile, platform: Platform, signals: &Signals) -> Attribution {
    let holds_microphone = signals
        .captures
        .iter()
        .any(|c| c.active && profile.matches(platform, &c.executable));
    let camera = match platform {
        // PipeWire attributes camera streams to their app; Teams is never
        // the app on Linux, so this stays unknown there too.
        Platform::Linux => {
            let ours: Vec<&CameraStream> = signals
                .cameras
                .iter()
                .filter(|c| profile.matches(platform, &c.executable))
                .collect();
            if ours.is_empty() {
                Tri::Unknown
            } else {
                Tri::Known(ours.iter().any(|c| c.active))
            }
        }
        // macOS: a camera in use is not attributed to an app; Windows has
        // no supported camera-in-use signal. Never the call's camera.
        Platform::MacOs | Platform::Windows => Tri::Unknown,
    };
    Attribution {
        holds_microphone,
        camera,
    }
}

/// The capabilities of a profile on a platform: detection only.
pub fn capabilities(profile: &Profile, platform: Platform) -> CapabilitySet {
    let no_api = Support::Unsupported {
        reason: UnsupportedReason::ProviderHasNoApi,
    };
    let platform_missing = Support::Unsupported {
        reason: UnsupportedReason::PlatformNotSupported,
    };
    let detect = if profile.attributes_on(platform) {
        Support::Inferred
    } else {
        platform_missing
    };
    let (set, _) = CapabilitySet::from_reported(Capability::ALL.into_iter().map(|capability| {
        let support = match capability {
            // Holding the microphone says nothing about its mute state:
            // `MicReadState` is unsupported like every control below.
            Capability::DetectMeeting => detect,
            // Ringing, dialing and titles have no supported signal.
            Capability::DetectIncoming | Capability::DetectOutgoing | Capability::ReadTitle => {
                no_api
            }
            Capability::CameraReadState => platform_missing,
            _ => no_api,
        };
        (capability, support)
    }));
    set
}

/// The provider: samples its source when polled, and reports the profile's
/// app as one call (`os-signals:<profile>`).
pub struct OsSignalsProvider {
    profile: Profile,
    source: Box<dyn SignalSource>,
    session: Option<ProviderSession>,
    /// Whether the app held the microphone at the last sample.
    in_call: bool,
    /// The last source failure, reported once.
    failing: bool,
    /// `Connected` is still to be reported for the session `connect`
    /// started: the first poll after it reports it (the contract:
    /// `Connected` follows from `poll`).
    announce: bool,
}

impl std::fmt::Debug for OsSignalsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsSignalsProvider")
            .field("profile", &self.profile.id)
            .field("in_call", &self.in_call)
            .finish_non_exhaustive()
    }
}

impl OsSignalsProvider {
    pub fn new(profile: Profile, source: Box<dyn SignalSource>) -> Self {
        Self {
            profile,
            source,
            session: None,
            in_call: false,
            failing: false,
            announce: false,
        }
    }

    fn key(&self) -> CallKey {
        CallKey::new(format!("os-signals:{}", self.profile.id)).expect("a valid key")
    }
}

impl CallProvider for OsSignalsProvider {
    fn id(&self) -> ProviderId {
        ProviderId::new(ProviderId::OS_SIGNALS).expect("a valid id")
    }

    fn discover(&mut self, _now: Instant) -> Discovery {
        let platform = self.source.platform();
        let availability = if self.profile.attributes_on(platform) {
            Availability::Available
        } else {
            Availability::Unsupported {
                reason: UnsupportedReason::PlatformNotSupported,
            }
        };
        Discovery {
            availability,
            capabilities: capabilities(&self.profile, platform),
            app_version: AppVersion::new("os-signals-1"),
        }
    }

    fn connect(&mut self, cx: &ProviderCx) -> Result<(), ProviderError> {
        if !cx.consent.observe {
            return Err(ProviderError::ConsentNotGiven);
        }
        self.session = Some(cx.session);
        self.announce = true;
        Ok(())
    }

    fn disconnect(&mut self) {
        self.session = None;
        self.in_call = false;
        self.failing = false;
        self.announce = false;
    }

    fn poll(&mut self, now: Instant) -> Vec<ProviderEvent> {
        let Some(session) = self.session else {
            return Vec::new();
        };
        let mut events = Vec::new();
        if std::mem::take(&mut self.announce) {
            events.push(ProviderEvent::Connected { session });
        }
        let platform = self.source.platform();
        match self.source.sample(now) {
            Err(_) => {
                if !self.failing {
                    self.failing = true;
                    self.in_call = false;
                    events.push(ProviderEvent::Disconnected {
                        reason: DisconnectReason::Network,
                    });
                }
                return events;
            }
            Ok(signals) => {
                if std::mem::take(&mut self.failing) {
                    events.push(ProviderEvent::Connected { session });
                }
                let attribution = attribute(&self.profile, platform, &signals);
                if attribution.holds_microphone {
                    self.in_call = true;
                    events.push(ProviderEvent::Call(CallObservation {
                        key: self.key(),
                        kind: CallKind::Unknown,
                        phase: ObservedPhase::Connected,
                        direction: None,
                        // Holding the microphone is not "not muted".
                        mic_muted: Tri::Unknown,
                        camera_on: attribution.camera,
                        title: None,
                        source: Evidence::OsSignal,
                        observed_at: now,
                    }));
                } else if std::mem::take(&mut self.in_call) {
                    events.push(ProviderEvent::CallGone { key: self.key() });
                }
            }
        }
        events
    }

    fn command(
        &mut self,
        _command: CallCommand,
        _token: CommandToken,
    ) -> Result<(), CommandRefusal> {
        // No supported way to control the app: nothing is ever sent.
        Err(CommandRefusal::Unsupported(
            UnsupportedReason::ProviderHasNoApi,
        ))
    }
}

// ------------------------------------------------------------------ parsers

/// `pw-dump` output (`PipeWire`'s JSON dump): the audio capture streams
/// (`Stream/Input/Audio`) and camera streams (`Stream/Input/Video`) with
/// the binary of the app that made them. Only `application.process.binary`
/// is read (a file name), never the command line.
pub fn parse_pw_dump(json: &str) -> Result<Signals, SourceError> {
    #[derive(Deserialize)]
    struct Object {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default)]
        info: Option<Info>,
    }
    #[derive(Deserialize)]
    struct Info {
        #[serde(default)]
        state: Option<String>,
        #[serde(default)]
        props: Props,
    }
    /// The two properties read; every other one (`media.name`, which may
    /// hold a tab's or meeting's title, the process id, the user) is
    /// skipped without being kept.
    #[derive(Deserialize, Default)]
    struct Props {
        #[serde(default, rename = "media.class")]
        media_class: Option<String>,
        #[serde(default, rename = "application.process.binary")]
        binary: Option<String>,
    }
    let objects: Vec<Object> = serde_json::from_str(bounded(json, "pw-dump-invalid")?)
        .map_err(|_| SourceError(FailureCode::new("pw-dump-invalid")))?;
    let mut signals = Signals::default();
    for object in objects {
        if object.kind != "PipeWire:Interface:Node" {
            continue;
        }
        let Some(info) = object.info else { continue };
        let class = info.props.media_class.as_deref().unwrap_or_default();
        let executable = info
            .props
            .binary
            .as_deref()
            .map(file_name)
            .unwrap_or_default()
            .to_owned();
        let active = info.state.as_deref() == Some("running");
        match class {
            "Stream/Input/Audio" => signals.captures.push(Capture { executable, active }),
            "Stream/Input/Video" => {
                if active {
                    signals.camera_in_use = Tri::Known(true);
                }
                signals.cameras.push(CameraStream { executable, active });
            }
            _ => {}
        }
    }
    if signals.camera_in_use == Tri::Unknown {
        signals.camera_in_use = Tri::Known(false);
    }
    Ok(signals)
}

/// A WASAPI capture session as the Windows source records it: the
/// session's process image name and `AudioSessionState`. Read only by
/// [`parse_wasapi_sessions`]; it has no `Debug`, so the image path is never
/// logged.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WasapiSession {
    process_image: String,
    /// `active`, `inactive` or `expired`.
    state: String,
}

/// The Windows source's capture sessions. The camera has no supported
/// signal on Windows: unknown.
pub fn parse_wasapi_sessions(json: &str) -> Result<Signals, SourceError> {
    let sessions: Vec<WasapiSession> = serde_json::from_str(bounded(json, "wasapi-invalid")?)
        .map_err(|_| SourceError(FailureCode::new("wasapi-invalid")))?;
    Ok(Signals {
        captures: sessions
            .into_iter()
            .map(|s| Capture {
                executable: file_name(&s.process_image).to_owned(),
                active: s.state.eq_ignore_ascii_case("active"),
            })
            .collect(),
        cameras: Vec::new(),
        camera_in_use: Tri::Unknown,
    })
}

/// The macOS source's record: the Core Audio process objects
/// (`kAudioHardwarePropertyProcessObjectList`) with their executable name
/// and `kAudioProcessPropertyIsRunningInput`, and
/// `AVCaptureDevice.isInUseByAnotherApplication` of the cameras. Read only
/// by [`parse_core_audio`]; no `Debug`, so no name is ever logged.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAudioRecord {
    processes: Vec<CoreAudioProcess>,
    #[serde(default)]
    cameras_in_use: Option<Vec<bool>>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreAudioProcess {
    executable: String,
    is_running_input: bool,
}

pub fn parse_core_audio(json: &str) -> Result<Signals, SourceError> {
    let record: CoreAudioRecord = serde_json::from_str(bounded(json, "core-audio-invalid")?)
        .map_err(|_| SourceError(FailureCode::new("core-audio-invalid")))?;
    Ok(Signals {
        captures: record
            .processes
            .into_iter()
            .map(|p| Capture {
                executable: file_name(&p.executable).to_owned(),
                active: p.is_running_input,
            })
            .collect(),
        cameras: Vec::new(),
        camera_in_use: record
            .cameras_in_use
            .map_or(Tri::Unknown, |c| Tri::Known(c.into_iter().any(|b| b))),
    })
}

/// Largest record a parser reads (a busy `PipeWire` graph's dump is well
/// under it).
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

/// `json` if it is at most [`MAX_RECORD_BYTES`], else the parser's code.
fn bounded<'a>(json: &'a str, code: &'static str) -> Result<&'a str, SourceError> {
    if json.len() > MAX_RECORD_BYTES {
        Err(SourceError(FailureCode::new(code)))
    } else {
        Ok(json)
    }
}

/// The file name of a path (`C:\…\ms-teams.exe`, `/usr/bin/chrome`).
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
