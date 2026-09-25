//! Core Audio implementation of the [`tunic_engine`] platform contract using
//! processing definitions from [`tunic_dsp`].

mod devices;
mod route;

use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2_core_audio::{
    AudioObjectAddPropertyListenerBlock, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListenerBlock, kAudioDevicePropertyNominalSampleRate,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};
use tunic_dsp::Equalizer;
use tunic_engine::{
    ActiveRoute, AudioPlatform, BypassControl, DeviceId, PlatformError, PlatformEvent,
    PlatformEventSink, PlatformState, ProcessedOutputFormat, ProcessedOutputSink,
    TelemetryPublisher,
};

use crate::devices::{
    channel_count, default_output_id, device_name, device_uid, list_output_devices, sample_rate,
};
use crate::route::Route;

pub struct CoreAudioPlatform {
    route: Option<Route>,
    default_output_listener: Option<PropertyListener>,
    sample_rate_listener: Option<PropertyListener>,
    events: Option<PlatformEventSink>,
    bypass: BypassControl,
    output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    telemetry: Option<TelemetryPublisher>,
    equalizer: Equalizer,
}

impl CoreAudioPlatform {
    #[must_use]
    pub fn new() -> Self {
        Self {
            route: None,
            default_output_listener: None,
            sample_rate_listener: None,
            events: None,
            bypass: BypassControl::default(),
            output_sink: None,
            telemetry: None,
            equalizer: Equalizer::identity(),
        }
    }

    fn build_default_route(&mut self) -> Result<PlatformState, PlatformError> {
        let output_id = default_output_id()?;
        let events = self
            .events
            .as_ref()
            .ok_or_else(|| PlatformError::new("platform event sink is not configured"))?;
        let sample_rate_listener = PropertyListener::new(
            output_id,
            sample_rate_property(),
            events.clone(),
            PlatformEvent::OutputSampleRateChanged,
        )?;
        let sample_rate_hz = sample_rate(output_id)?;
        let route = ActiveRoute {
            device_id: DeviceId::new(device_uid(output_id)?),
            device_name: device_name(output_id)?,
            sample_rate_hz,
            channels: channel_count(output_id)?,
        };
        if let Some(output_sink) = &self.output_sink {
            output_sink
                .configure(ProcessedOutputFormat {
                    sample_rate_hz: route.sample_rate_hz,
                    channels: 2,
                })
                .map_err(|error| PlatformError::new(error.to_string()))?;
        }
        if let Some(mut previous_route) = self.route.take() {
            previous_route.stop()?;
        }
        let telemetry = self
            .telemetry
            .as_ref()
            .ok_or_else(|| PlatformError::new("telemetry publisher is not configured"))?;
        let devices = list_output_devices()?;
        self.route = Some(Route::start(
            output_id,
            sample_rate_hz,
            self.bypass.clone(),
            self.output_sink.clone(),
            telemetry.clone(),
            &self.equalizer,
        )?);
        self.sample_rate_listener = Some(sample_rate_listener);
        Ok(PlatformState { route, devices })
    }

    fn shutdown_resources(&mut self) -> Result<(), PlatformError> {
        let default_listener_result = self
            .default_output_listener
            .take()
            .map_or(Ok(()), |mut listener| listener.remove());
        let sample_rate_listener_result = self
            .sample_rate_listener
            .take()
            .map_or(Ok(()), |mut listener| listener.remove());
        let route_result = self.route.take().map_or(Ok(()), |mut route| route.stop());
        combine_results(
            default_listener_result,
            combine_results(sample_rate_listener_result, route_result),
        )
    }
}

impl Default for CoreAudioPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioPlatform for CoreAudioPlatform {
    fn start(
        &mut self,
        events: PlatformEventSink,
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
        equalizer: &Equalizer,
        bypass: BypassControl,
    ) -> Result<PlatformState, PlatformError> {
        self.output_sink = output_sink;
        self.telemetry = Some(telemetry);
        self.equalizer = equalizer.clone();
        self.bypass = bypass;
        self.events = Some(events.clone());
        self.default_output_listener = Some(PropertyListener::new(
            kAudioObjectSystemObject as AudioObjectID,
            default_output_property(),
            events,
            PlatformEvent::DefaultOutputChanged,
        )?);
        match self.build_default_route() {
            Ok(state) => Ok(state),
            Err(start_error) => {
                let cleanup = self.shutdown_resources();
                Err(with_cleanup_error(start_error, cleanup))
            }
        }
    }

    fn rebuild_default_route(&mut self) -> Result<PlatformState, PlatformError> {
        self.build_default_route()
    }

    fn set_equalizer(&mut self, equalizer: &Equalizer) -> Result<(), PlatformError> {
        let route = self
            .route
            .as_ref()
            .ok_or_else(|| PlatformError::new("no active output route"))?;
        route.set_equalizer(equalizer)?;
        self.equalizer = equalizer.clone();
        Ok(())
    }

    fn shutdown(&mut self) -> Result<(), PlatformError> {
        self.shutdown_resources()
    }
}

impl Drop for CoreAudioPlatform {
    fn drop(&mut self) {
        let _ = self.shutdown_resources();
    }
}

type ListenerBlock = RcBlock<dyn Fn(u32, NonNull<AudioObjectPropertyAddress>)>;

struct PropertyListener {
    object_id: AudioObjectID,
    property: AudioObjectPropertyAddress,
    block: ListenerBlock,
    registered: bool,
}

impl PropertyListener {
    fn new(
        object_id: AudioObjectID,
        property: AudioObjectPropertyAddress,
        events: PlatformEventSink,
        event: PlatformEvent,
    ) -> Result<Self, PlatformError> {
        let block = RcBlock::new(
            move |_address_count: u32, _addresses: NonNull<AudioObjectPropertyAddress>| {
                let _ = events.send(event);
            },
        );
        // SAFETY: Core Audio copies the block and retains its captures until matching removal.
        let status = unsafe {
            AudioObjectAddPropertyListenerBlock(
                object_id,
                NonNull::from(&property),
                None,
                RcBlock::as_ptr(&block),
            )
        };
        check_status("observe audio property", status)?;
        Ok(Self {
            object_id,
            property,
            block,
            registered: true,
        })
    }

    fn remove(&mut self) -> Result<(), PlatformError> {
        if !self.registered {
            return Ok(());
        }
        // SAFETY: this is the same block, address, and queue used during registration.
        let status = unsafe {
            AudioObjectRemovePropertyListenerBlock(
                self.object_id,
                NonNull::from(&self.property),
                None,
                RcBlock::as_ptr(&self.block),
            )
        };
        check_status("stop observing audio property", status)?;
        self.registered = false;
        Ok(())
    }
}

impl Drop for PropertyListener {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

pub(crate) const fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn default_output_property() -> AudioObjectPropertyAddress {
    address(
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyScopeGlobal,
    )
}

fn sample_rate_property() -> AudioObjectPropertyAddress {
    address(
        kAudioDevicePropertyNominalSampleRate,
        kAudioObjectPropertyScopeOutput,
    )
}

pub(crate) fn check_status(operation: &str, status: i32) -> Result<(), PlatformError> {
    if status == 0 {
        Ok(())
    } else {
        Err(status_error(operation, status))
    }
}

pub(crate) fn status_error(operation: &str, status: i32) -> PlatformError {
    let bytes = (status as u32).to_be_bytes();
    let detail = if bytes.iter().all(u8::is_ascii_graphic) {
        format!("'{}'", String::from_utf8_lossy(&bytes))
    } else {
        status.to_string()
    };
    PlatformError::new(format!("{operation} failed with OSStatus {detail}"))
}

fn combine_results(
    first: Result<(), PlatformError>,
    second: Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(PlatformError::new(format!("{first}; {second}"))),
    }
}

fn with_cleanup_error(primary: PlatformError, cleanup: Result<(), PlatformError>) -> PlatformError {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => PlatformError::new(format!("{primary}; cleanup also failed: {cleanup}")),
    }
}
