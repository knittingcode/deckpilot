//! In-memory device backend for automated tests and hardware-free development.
//!
//! [`MockBackend`] is cheaply cloneable; clones share state, so a test can keep
//! one clone to plug devices, inject input and inspect rendered frames while
//! the code under test owns another.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{
    ControlId, ControlInput, DeviceBackend, DeviceError, DeviceHandle, DeviceId, DeviceModel,
    DiscoveredDevice, Frame,
};

#[derive(Debug, Default)]
struct MockDevice {
    model: Option<DeviceModel>,
    connected: bool,
    /// Incremented on every plug so handles from a previous connection fail.
    generation: u64,
    input: VecDeque<(ControlId, ControlInput)>,
    pending: BTreeMap<ControlId, Option<Frame>>,
    frames: BTreeMap<ControlId, Frame>,
    image_writes: usize,
    /// Images written per control, for checks that unrelated keys are
    /// left alone.
    writes: BTreeMap<ControlId, usize>,
    brightness: Option<u8>,
    /// Calls to [`DeviceBackend::open`], failed ones included.
    open_attempts: usize,
    /// Injected faults ([`MockBackend::fail_open`] and others).
    faults: Faults,
}

/// Faults injected into one simulated device, to test how the agent
/// copes with hardware that misbehaves.
#[derive(Debug, Default)]
struct Faults {
    /// Opens fail with this error...
    open: Option<DeviceError>,
    /// ...this many more times (`None`: until cleared).
    open_times: Option<usize>,
    /// Each opened handle fails with this error once it has written this
    /// many reports (an image or a brightness each), and from then on.
    after_writes: Option<(usize, DeviceError)>,
}

#[derive(Debug, Default)]
struct Shared {
    devices: Mutex<BTreeMap<DeviceId, MockDevice>>,
    changed: Condvar,
}

/// Simulated devices with shared, inspectable state.
#[derive(Debug, Clone, Default)]
pub struct MockBackend {
    shared: Arc<Shared>,
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<DeviceId, MockDevice>> {
        self.shared
            .devices
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Attaches a device of `model` with the given serial number. Like a
    /// real device after a power cycle, it starts blank and with its own
    /// default brightness (reported as `None`).
    pub fn plug(&self, model: &DeviceModel, serial: &str) -> DeviceId {
        let id = DeviceId {
            model: model.id.clone(),
            serial: serial.into(),
        };
        let mut devices = self.lock();
        let device = devices.entry(id.clone()).or_default();
        device.model = Some(model.clone());
        device.connected = true;
        device.generation += 1;
        device.input.clear();
        device.frames.clear();
        device.pending.clear();
        device.brightness = None;
        drop(devices);
        self.shared.changed.notify_all();
        id
    }

    /// The device re-enumerates without being unplugged, as after a USB
    /// reset or when it comes back under another path: open handles
    /// fail, it is listed again under the same serial, blank and at its
    /// default brightness. Faults stay.
    pub fn reenumerate(&self, id: &DeviceId) {
        let model = self.lock().get(id).and_then(|device| device.model.clone());
        if let Some(model) = model {
            self.plug(&model, &id.serial);
        }
    }

    /// Every open of `id` fails with `error` until [`MockBackend::clear_faults`].
    /// The device stays listed, like one the user may not open.
    pub fn fail_open(&self, id: &DeviceId, error: DeviceError) {
        self.with_faults(id, |faults| {
            faults.open = Some(error);
            faults.open_times = None;
        });
    }

    /// The next `times` opens of `id` fail with `error`; later ones work.
    pub fn fail_open_times(&self, id: &DeviceId, error: DeviceError, times: usize) {
        self.with_faults(id, |faults| {
            faults.open = Some(error);
            faults.open_times = Some(times);
        });
    }

    /// Every handle opened from now on fails with `error` after `writes`
    /// reports (images or brightness), like a device on a marginal link
    /// that opens and then drops. Until [`MockBackend::clear_faults`].
    pub fn fail_after_writes(&self, id: &DeviceId, writes: usize, error: DeviceError) {
        self.with_faults(id, |faults| faults.after_writes = Some((writes, error)));
    }

    /// Removes the faults of `id`.
    pub fn clear_faults(&self, id: &DeviceId) {
        self.with_faults(id, |faults| *faults = Faults::default());
    }

    /// Opens of `id` so far, failed ones included.
    pub fn open_attempts(&self, id: &DeviceId) -> usize {
        self.lock().get(id).map_or(0, |device| device.open_attempts)
    }

    fn with_faults(&self, id: &DeviceId, f: impl FnOnce(&mut Faults)) {
        if let Some(device) = self.lock().get_mut(id) {
            f(&mut device.faults);
        }
        self.shared.changed.notify_all();
    }

    /// Detaches a device. Open handles report [`DeviceError::Disconnected`].
    pub fn unplug(&self, id: &DeviceId) {
        if let Some(device) = self.lock().get_mut(id) {
            device.connected = false;
        }
        self.shared.changed.notify_all();
    }

    /// Queues an input as if it was produced by the hardware.
    pub fn send_input(&self, id: &DeviceId, control: ControlId, input: ControlInput) {
        if let Some(device) = self.lock().get_mut(id) {
            device.input.push_back((control, input));
        }
        self.shared.changed.notify_all();
    }

    /// Queues several inputs at once: the device's reader takes them in
    /// one read, so they all carry the same read time. Tests use this for
    /// sequences whose gesture must not depend on scheduling (a click held
    /// for no time at all, a double click with no gap).
    pub fn send_inputs(&self, id: &DeviceId, inputs: &[(ControlId, ControlInput)]) {
        if let Some(device) = self.lock().get_mut(id) {
            device.input.extend(inputs.iter().copied());
        }
        self.shared.changed.notify_all();
    }

    /// Queues a press followed by a release.
    pub fn tap(&self, id: &DeviceId, control: ControlId) {
        self.send_input(id, control, ControlInput::Press);
        self.send_input(id, control, ControlInput::Release);
    }

    /// Frames currently shown on the device.
    pub fn frames(&self, id: &DeviceId) -> BTreeMap<ControlId, Frame> {
        self.lock()
            .get(id)
            .map(|device| device.frames.clone())
            .unwrap_or_default()
    }

    /// Total number of images written since the device was created.
    pub fn image_writes(&self, id: &DeviceId) -> usize {
        self.lock().get(id).map_or(0, |device| device.image_writes)
    }

    /// Images written to each control since the device was created.
    pub fn control_writes(&self, id: &DeviceId) -> BTreeMap<ControlId, usize> {
        self.lock()
            .get(id)
            .map(|device| device.writes.clone())
            .unwrap_or_default()
    }

    pub fn brightness(&self, id: &DeviceId) -> Option<u8> {
        self.lock().get(id).and_then(|device| device.brightness)
    }

    /// Blocks until `predicate` holds for the device state or `timeout` passes.
    /// Returns whether the predicate was satisfied.
    pub fn wait_for(
        &self,
        id: &DeviceId,
        timeout: Duration,
        predicate: impl Fn(&BTreeMap<ControlId, Frame>, usize) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let mut devices = self.lock();
        loop {
            if let Some(device) = devices.get(id)
                && predicate(&device.frames, device.image_writes)
            {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            devices = self
                .shared
                .changed
                .wait_timeout(devices, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl DeviceBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, DeviceError> {
        Ok(self
            .lock()
            .iter()
            .filter(|(_, device)| device.connected)
            .map(|(id, _)| DiscoveredDevice {
                id: id.clone(),
                model: id.model.clone(),
            })
            .collect())
    }

    fn open(&mut self, id: &DeviceId) -> Result<Box<dyn DeviceHandle>, DeviceError> {
        let mut devices = self.lock();
        let device = devices
            .get_mut(id)
            .filter(|d| d.connected)
            .ok_or(DeviceError::Disconnected)?;
        device.open_attempts += 1;
        if let Some(error) = device.faults.open.clone() {
            match &mut device.faults.open_times {
                None => return Err(error),
                Some(0) => device.faults.open = None,
                Some(left) => {
                    *left -= 1;
                    if *left == 0 {
                        device.faults.open = None;
                        device.faults.open_times = None;
                    }
                    return Err(error);
                }
            }
        }
        let model = device.model.clone().ok_or(DeviceError::Disconnected)?;
        let generation = device.generation;
        let fail_after = device.faults.after_writes.clone();
        drop(devices);
        self.shared.changed.notify_all();
        Ok(Box::new(MockHandle {
            backend: self.clone(),
            id: id.clone(),
            model,
            generation,
            fail_after,
            written: 0,
            failed: None,
        }))
    }
}

#[derive(Debug)]
struct MockHandle {
    backend: MockBackend,
    id: DeviceId,
    model: DeviceModel,
    generation: u64,
    /// [`MockBackend::fail_after_writes`] when this handle was opened.
    fail_after: Option<(usize, DeviceError)>,
    /// Reports this handle wrote.
    written: usize,
    /// The injected failure happened: every later call fails too.
    failed: Option<DeviceError>,
}

impl MockHandle {
    /// Counts `reports` about to be written; fails once the injected limit
    /// is passed, and from then on.
    fn write(&mut self, reports: usize) -> Result<(), DeviceError> {
        self.check_failed()?;
        if let Some((limit, error)) = &self.fail_after
            && self.written.saturating_add(reports) > *limit
        {
            self.failed = Some(error.clone());
            return Err(error.clone());
        }
        self.written = self.written.saturating_add(reports);
        Ok(())
    }

    fn check_failed(&self) -> Result<(), DeviceError> {
        self.failed.clone().map_or(Ok(()), Err)
    }

    fn with_device<T>(
        &self,
        f: impl FnOnce(&mut MockDevice) -> Result<T, DeviceError>,
    ) -> Result<T, DeviceError> {
        let mut devices = self.backend.lock();
        match devices.get_mut(&self.id) {
            Some(device) if device.connected && device.generation == self.generation => f(device),
            _ => Err(DeviceError::Disconnected),
        }
    }

    fn check_display(&self, control: ControlId) -> Result<(), DeviceError> {
        if self.model.capabilities.has_display(control) {
            Ok(())
        } else {
            Err(DeviceError::InvalidControl(control))
        }
    }
}

impl DeviceHandle for MockHandle {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn read_input(
        &mut self,
        timeout: Duration,
    ) -> Result<Vec<(ControlId, ControlInput)>, DeviceError> {
        self.check_failed()?;
        let deadline = Instant::now() + timeout;
        let mut devices = self.backend.lock();
        loop {
            match devices.get_mut(&self.id) {
                Some(device) if device.connected && device.generation == self.generation => {
                    if !device.input.is_empty() {
                        return Ok(device.input.drain(..).collect());
                    }
                }
                _ => return Err(DeviceError::Disconnected),
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(Vec::new());
            }
            devices = self
                .backend
                .shared
                .changed
                .wait_timeout(devices, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn set_image(&mut self, control: ControlId, frame: &Frame) -> Result<(), DeviceError> {
        self.check_failed()?;
        self.check_display(control)?;
        self.with_device(|device| {
            device.pending.insert(control, Some(frame.clone()));
            Ok(())
        })
    }

    fn clear_image(&mut self, control: ControlId) -> Result<(), DeviceError> {
        self.check_failed()?;
        self.check_display(control)?;
        self.with_device(|device| {
            device.pending.insert(control, None);
            Ok(())
        })
    }

    fn set_brightness(&mut self, percent: u8) -> Result<(), DeviceError> {
        self.write(1)?;
        self.with_device(|device| {
            device.brightness = Some(percent.min(100));
            Ok(())
        })?;
        self.backend.shared.changed.notify_all();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), DeviceError> {
        let queued = self.with_device(|device| Ok(device.pending.len()))?;
        if let Err(error) = self.write(queued) {
            // The reports are lost, as on a device that dropped.
            let _ = self.with_device(|device| {
                device.pending.clear();
                Ok(())
            });
            return Err(error);
        }
        self.with_device(|device| {
            for (control, frame) in std::mem::take(&mut device.pending) {
                device.image_writes += 1;
                *device.writes.entry(control).or_default() += 1;
                match frame {
                    Some(frame) => device.frames.insert(control, frame),
                    None => device.frames.remove(&control),
                };
            }
            Ok(())
        })?;
        self.backend.shared.changed.notify_all();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models;

    #[test]
    fn discovery_follows_plug_state() {
        let mut backend = MockBackend::new();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        assert_eq!(backend.discover().unwrap().len(), 1);
        backend.unplug(&id);
        assert_eq!(backend.discover().unwrap(), vec![]);
        assert!(backend.open(&id).is_err());
    }

    #[test]
    fn handle_reads_input_and_writes_frames() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        let mut handle = backend.open(&id).unwrap();

        assert_eq!(handle.read_input(Duration::from_millis(1)).unwrap(), vec![]);
        probe.tap(&id, ControlId::Key { index: 3 });
        let events = handle.read_input(Duration::from_millis(100)).unwrap();
        assert_eq!(events.len(), 2);

        let key = ControlId::Key { index: 3 };
        handle
            .set_image(key, &Frame::filled(72, 72, [255, 0, 0]))
            .unwrap();
        assert_eq!(probe.frames(&id).len(), 0, "images apply on flush");
        handle.flush().unwrap();
        assert_eq!(probe.frames(&id)[&key].pixel(0, 0), Some([255, 0, 0]));
        assert!(
            handle
                .set_image(ControlId::Key { index: 15 }, &Frame::filled(1, 1, [0; 3]))
                .is_err()
        );

        handle.set_brightness(150).unwrap();
        assert_eq!(probe.brightness(&id), Some(100));
    }

    #[test]
    fn injected_open_failures_and_counts() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        for error in [
            DeviceError::Io("permission denied".into()),
            DeviceError::Unsupported("busy".into()),
            DeviceError::Disconnected,
        ] {
            probe.fail_open(&id, error.clone());
            assert_eq!(backend.open(&id).err(), Some(error.clone()));
            assert_eq!(backend.open(&id).err(), Some(error));
            assert_eq!(backend.discover().unwrap().len(), 1, "still listed");
        }
        probe.clear_faults(&id);
        assert!(backend.open(&id).is_ok());
        assert_eq!(probe.open_attempts(&id), 7);

        probe.fail_open_times(&id, DeviceError::Io("busy".into()), 2);
        assert!(backend.open(&id).is_err());
        assert!(backend.open(&id).is_err());
        assert!(backend.open(&id).is_ok());
        assert!(backend.open(&id).is_ok());
    }

    #[test]
    fn handles_fail_after_the_injected_writes() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        probe.fail_after_writes(&id, 2, DeviceError::Io("broken pipe".into()));
        let mut handle = backend.open(&id).unwrap();
        handle.set_brightness(50).unwrap();
        let frame = Frame::filled(72, 72, [1, 2, 3]);
        handle
            .set_image(ControlId::Key { index: 0 }, &frame)
            .unwrap();
        handle.flush().unwrap();
        handle
            .set_image(ControlId::Key { index: 1 }, &frame)
            .unwrap();
        let broken = Err(DeviceError::Io("broken pipe".into()));
        assert_eq!(handle.flush(), broken);
        assert_eq!(
            handle.read_input(Duration::from_millis(1)).map(|_| ()),
            broken
        );
        // A new handle fails the same way; without the fault it works.
        let mut again = backend.open(&id).unwrap();
        assert!(again.set_brightness(1).is_ok());
        assert!(again.set_brightness(1).is_ok());
        assert!(again.set_brightness(1).is_err());
        probe.clear_faults(&id);
        let mut fine = backend.open(&id).unwrap();
        for _ in 0..10 {
            fine.set_brightness(1).unwrap();
        }
    }

    #[test]
    fn reenumerating_keeps_the_serial_and_fails_old_handles() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        let mut old = backend.open(&id).unwrap();
        old.set_brightness(40).unwrap();
        probe.reenumerate(&id);
        assert_eq!(
            old.read_input(Duration::from_millis(1)),
            Err(DeviceError::Disconnected)
        );
        assert_eq!(probe.brightness(&id), None, "starts at its default");
        assert_eq!(backend.discover().unwrap()[0].id, id);
        assert!(backend.open(&id).is_ok());
    }

    #[test]
    fn stale_handles_fail_after_replug() {
        let mut backend = MockBackend::new();
        let probe = backend.clone();
        let id = backend.plug(&models::stream_deck_mk2(), "A1");
        let mut old = backend.open(&id).unwrap();
        probe.unplug(&id);
        assert_eq!(
            old.read_input(Duration::from_millis(1)),
            Err(DeviceError::Disconnected)
        );
        probe.plug(&models::stream_deck_mk2(), "A1");
        assert_eq!(
            old.read_input(Duration::from_millis(1)),
            Err(DeviceError::Disconnected)
        );
        assert!(
            backend
                .open(&id)
                .unwrap()
                .read_input(Duration::from_millis(1))
                .is_ok()
        );
    }
}
