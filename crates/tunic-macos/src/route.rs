use std::cell::{Cell, RefCell};
use std::f32::consts::TAU;
use std::ffi::{CStr, c_void};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_audio::{
    AudioDeviceCreateIOProcIDWithBlock, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID,
    AudioDeviceStart, AudioDeviceStop, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectID, CATapDescription,
    CATapMuteBehavior, kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceMainSubDeviceKey, kAudioAggregateDeviceNameKey,
    kAudioAggregateDeviceSubDeviceListKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioAggregateDriftCompensationMaxQuality, kAudioDevicePropertyDeviceIsAlive,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyScopeGlobal,
    kAudioObjectSystemObject, kAudioSubDeviceUIDKey, kAudioSubTapDriftCompensationKey,
    kAudioSubTapDriftCompensationQualityKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObject, NSString, NSUUID};
use rustfft::{Fft, FftPlanner, num_complex::Complex32};
use tunic_dsp::{Equalizer, PreparedGraph};
use tunic_engine::{
    ChannelLevels, PlatformError, ProcessedOutputSink, SPECTRUM_BAND_COUNT,
    SPECTRUM_FREQUENCIES_HZ, Spectrum, StereoLevels, TelemetryFrame, TelemetryGeneration,
    TelemetryPublisher,
};

use crate::devices::{device_uid, input_stream_count};
use crate::{address, check_status};

const SCRATCH_FRAME_CAPACITY: usize = 16_384;
const TELEMETRY_UPDATES_PER_SECOND: f64 = 30.0;
const SPECTRUM_MAX_BIN_WIDTH_HZ: f64 = 6.0;
const PEAK_DECAY_DB_PER_SECOND: f32 = 20.0;
const RMS_DECAY_DB_PER_SECOND: f32 = 12.0;
const SPECTRUM_DECAY_DB_PER_SECOND: f32 = 24.0;
const SPECTRUM_ATTACK: f32 = 0.65;

type IoBlock = RcBlock<
    dyn Fn(
        NonNull<AudioTimeStamp>,
        NonNull<AudioBufferList>,
        NonNull<AudioTimeStamp>,
        NonNull<AudioBufferList>,
        NonNull<AudioTimeStamp>,
    ),
>;

pub(crate) struct Route {
    aggregate_id: AudioObjectID,
    io_proc_id: AudioDeviceIOProcID,
    tap_id: AudioObjectID,
    started: bool,
    active: Arc<AtomicBool>,
    sample_rate_hz: f64,
    graph_updates: Arc<GraphExchange>,
    _block: IoBlock,
    _description: Retained<CATapDescription>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TapBufferRange(NonZeroUsize);

impl TapBufferRange {
    fn from_stream_counts(aggregate: usize, physical: usize) -> Result<Self, PlatformError> {
        let count = aggregate.checked_sub(physical).and_then(NonZeroUsize::new);
        count.map(Self).ok_or_else(|| {
            PlatformError::new(format!(
                "aggregate input layout has no tap streams (aggregate={aggregate}, physical={physical})"
            ))
        })
    }

    fn select<'a>(&self, buffers: &'a [AudioBuffer]) -> Option<&'a [AudioBuffer]> {
        let count = self.0.get();
        let start = buffers.len().checked_sub(count)?;
        Some(&buffers[start..])
    }
}

impl Route {
    pub(crate) fn start(
        output_id: AudioObjectID,
        sample_rate_hz: f64,
        bypassed: Arc<AtomicBool>,
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
        equalizer: &Equalizer,
    ) -> Result<Self, PlatformError> {
        Self::start_inner(
            output_id,
            sample_rate_hz,
            bypassed,
            output_sink,
            telemetry,
            equalizer,
        )
    }

    fn start_inner(
        output_id: AudioObjectID,
        sample_rate_hz: f64,
        bypassed: Arc<AtomicBool>,
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
        equalizer: &Equalizer,
    ) -> Result<Self, PlatformError> {
        let prepared_graph = PreparedGraph::prepare(equalizer, sample_rate_hz)
            .map_err(|error| PlatformError::new(error.to_string()))?;
        let output_uid = device_uid(output_id)?;
        let excluded_process = current_process_object().into_iter().collect::<Vec<_>>();
        let process_numbers = excluded_process
            .iter()
            .map(|id| NSNumber::numberWithUnsignedInt(*id))
            .collect::<Vec<_>>();
        let processes = NSArray::from_retained_slice(&process_numbers);
        let output_uid_string = NSString::from_str(&output_uid);
        // SAFETY: all Objective-C arguments remain alive through these calls.
        let description = unsafe {
            let description = CATapDescription::initExcludingProcesses_andDeviceUID_withStream(
                CATapDescription::alloc(),
                &processes,
                &output_uid_string,
                0,
            );
            description.setName(&NSString::from_str("Tunic system audio"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Muted);
            description
        };
        // SAFETY: description is a live CATapDescription.
        let tap_uid = unsafe { description.UUID().UUIDString() };

        let mut tap_id = 0;
        // SAFETY: description and tap_id are valid for the duration of the call.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap_id) };
        check_status("create process tap", status)?;
        if let Err(format_error) = validate_tap_format(tap_id) {
            return Err(with_cleanup_error(format_error, destroy_tap(tap_id)));
        }

        let aggregate_id = match create_aggregate(&output_uid_string, &tap_uid) {
            Ok(id) => id,
            Err(error) => {
                return Err(with_cleanup_error(error, destroy_tap(tap_id)));
            }
        };
        if let Err(error) = wait_until_alive(aggregate_id) {
            return Err(with_cleanup_error(
                error,
                destroy_aggregate_and_tap(aggregate_id, tap_id),
            ));
        }

        let tap_buffers = match (|| {
            let physical_input_streams = input_stream_count(output_id)?;
            let aggregate_input_streams = input_stream_count(aggregate_id)?;
            TapBufferRange::from_stream_counts(aggregate_input_streams, physical_input_streams)
        })() {
            Ok(range) => range,
            Err(error) => {
                return Err(with_cleanup_error(
                    error,
                    destroy_aggregate_and_tap(aggregate_id, tap_id),
                ));
            }
        };

        let active = Arc::new(AtomicBool::new(true));
        let callback_active = Arc::clone(&active);
        let scratch = RefCell::new(Box::new([0.0_f32; SCRATCH_FRAME_CAPACITY * 2]));
        let graph = RefCell::new(prepared_graph);
        let processed_output = RefCell::new(ProcessedOutputObservers::new(
            output_sink,
            telemetry,
            sample_rate_hz,
        ));
        let graph_updates = Arc::new(GraphExchange::new());
        let callback_graph_updates = Arc::clone(&graph_updates);
        let callback_bypassed = Cell::new(false);
        let block = RcBlock::new(
            move |_now: NonNull<AudioTimeStamp>,
                  input: NonNull<AudioBufferList>,
                  _input_time: NonNull<AudioTimeStamp>,
                  output: NonNull<AudioBufferList>,
                  _output_time: NonNull<AudioTimeStamp>| {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    // SAFETY: Core Audio owns both lists for the duration of this callback.
                    unsafe { zero_output(output.as_ptr()) };
                    if !callback_active.load(Ordering::Acquire) {
                        return;
                    }
                    let (Ok(mut scratch), Ok(mut graph), Ok(mut processed_output)) = (
                        scratch.try_borrow_mut(),
                        graph.try_borrow_mut(),
                        processed_output.try_borrow_mut(),
                    ) else {
                        return;
                    };
                    callback_graph_updates.install_latest(&mut graph);
                    let is_bypassed = bypassed.load(Ordering::Relaxed);
                    reset_graph_on_bypass(&mut graph, &callback_bypassed, is_bypassed);
                    render_audio(
                        input.as_ptr(),
                        output.as_ptr(),
                        tap_buffers,
                        &mut scratch[..],
                        &mut graph,
                        is_bypassed,
                        &mut processed_output,
                    );
                }));
            },
        );

        let mut io_proc_id = None;
        // SAFETY: the block is retained by Route until the IOProc is destroyed.
        let status = unsafe {
            AudioDeviceCreateIOProcIDWithBlock(
                NonNull::from(&mut io_proc_id),
                aggregate_id,
                None,
                RcBlock::as_ptr(&block),
            )
        };
        if status != 0 || io_proc_id.is_none() {
            let error = if status == 0 {
                PlatformError::new("create aggregate IOProc returned no IOProc")
            } else {
                crate::status_error("create aggregate IOProc", status)
            };
            return Err(with_cleanup_error(
                error,
                destroy_aggregate_and_tap(aggregate_id, tap_id),
            ));
        }

        let mut route = Self {
            aggregate_id,
            io_proc_id,
            tap_id,
            started: false,
            active,
            sample_rate_hz,
            graph_updates,
            _block: block,
            _description: description,
        };
        // SAFETY: aggregate_id and io_proc_id are a valid registered pair.
        let status = unsafe { AudioDeviceStart(aggregate_id, io_proc_id) };
        if status != 0 {
            let error = crate::status_error("start aggregate device", status);
            return Err(with_cleanup_error(error, route.stop()));
        }
        route.started = true;

        Ok(route)
    }

    pub(crate) fn set_equalizer(&self, equalizer: &Equalizer) -> Result<(), PlatformError> {
        let graph = PreparedGraph::prepare(equalizer, self.sample_rate_hz)
            .map_err(|error| PlatformError::new(error.to_string()))?;
        self.graph_updates.publish(graph);
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> Result<(), PlatformError> {
        self.active.store(false, Ordering::Release);
        let mut errors = Vec::new();

        if self.started && self.io_proc_id.is_some() {
            // SAFETY: aggregate_id and io_proc_id were created as a pair.
            let status = unsafe { AudioDeviceStop(self.aggregate_id, self.io_proc_id) };
            if status == 0 {
                self.started = false;
            } else {
                record_status(&mut errors, "stop aggregate device", status);
            }
        }

        if self.io_proc_id.is_some() {
            // SAFETY: aggregate_id and io_proc_id were created as a pair.
            let status = unsafe { AudioDeviceDestroyIOProcID(self.aggregate_id, self.io_proc_id) };
            if status == 0 {
                self.io_proc_id = None;
            } else {
                record_status(&mut errors, "destroy aggregate IOProc", status);
            }
        }

        if self.aggregate_id != 0 {
            // SAFETY: aggregate_id is owned by this route.
            let status = unsafe { AudioHardwareDestroyAggregateDevice(self.aggregate_id) };
            if status == 0 {
                self.aggregate_id = 0;
                self.io_proc_id = None;
                self.started = false;
            } else {
                record_status(&mut errors, "destroy aggregate device", status);
            }
        }

        if self.aggregate_id == 0 && self.tap_id != 0 {
            // SAFETY: tap_id is owned by this route.
            let status = unsafe { AudioHardwareDestroyProcessTap(self.tap_id) };
            if status == 0 {
                self.tap_id = 0;
            } else {
                record_status(&mut errors, "destroy process tap", status);
            }
        }

        errors_to_result(errors)
    }
}

fn reset_graph_on_bypass(graph: &mut PreparedGraph, was_bypassed: &Cell<bool>, is_bypassed: bool) {
    if is_bypassed && !was_bypassed.replace(is_bypassed) {
        graph.reset();
    }
}

struct GraphExchange {
    pending: AtomicPtr<GraphNode>,
    retired: AtomicPtr<GraphNode>,
}

impl GraphExchange {
    fn new() -> Self {
        Self {
            pending: AtomicPtr::new(std::ptr::null_mut()),
            retired: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// Publish from the non-real-time engine thread.
    fn publish(&self, graph: PreparedGraph) {
        self.reclaim_retired();
        let update = Box::into_raw(Box::new(GraphNode {
            graph,
            next: std::ptr::null_mut(),
        }));
        let superseded = self.pending.swap(update, Ordering::AcqRel);
        if !superseded.is_null() {
            // SAFETY: the producer won ownership of the pending node in the swap.
            drop(unsafe { Box::from_raw(superseded) });
        }
        self.reclaim_retired();
    }

    /// Install on the real-time callback without locking, allocating, or freeing.
    fn install_latest(&self, current: &mut PreparedGraph) {
        let update = self.pending.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if update.is_null() {
            return;
        }
        // SAFETY: the callback won ownership of the pending node in the swap.
        let node = unsafe { &mut *update };
        std::mem::swap(current, &mut node.graph);
        let mut head = self.retired.load(Ordering::Acquire);
        loop {
            node.next = head;
            match self.retired.compare_exchange_weak(
                head,
                update,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(actual) => head = actual,
            }
        }
    }

    fn reclaim_retired(&self) {
        let retired = self.retired.swap(std::ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: the producer owns the detached retired list.
        unsafe { drop_nodes(retired) };
    }
}

impl Drop for GraphExchange {
    fn drop(&mut self) {
        // The IOProc block owns an Arc, so final drop cannot run during a callback.
        unsafe {
            drop_nodes(*self.pending.get_mut());
            drop_nodes(*self.retired.get_mut());
        }
    }
}

struct GraphNode {
    graph: PreparedGraph,
    next: *mut GraphNode,
}

unsafe fn drop_nodes(mut node: *mut GraphNode) {
    while !node.is_null() {
        // SAFETY: the caller owns every node in this detached list.
        let boxed = unsafe { Box::from_raw(node) };
        node = boxed.next;
    }
}

impl Drop for Route {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn current_process_object() -> Option<AudioObjectID> {
    let property = address(
        kAudioHardwarePropertyTranslatePIDToProcessObject,
        kAudioObjectPropertyScopeGlobal,
    );
    let pid = std::process::id() as i32;
    let mut object = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&property),
            size_of::<i32>() as u32,
            (&raw const pid).cast::<c_void>(),
            NonNull::from(&mut size),
            NonNull::new_unchecked((&raw mut object).cast::<c_void>()),
        )
    };
    (status == 0 && object != 0).then_some(object)
}

fn validate_tap_format(tap_id: AudioObjectID) -> Result<(), PlatformError> {
    let property = address(kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal);
    let mut format = std::mem::MaybeUninit::<AudioStreamBasicDescription>::uninit();
    let mut size = size_of::<AudioStreamBasicDescription>() as u32;
    // SAFETY: format has enough storage and is only read after a successful call.
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            NonNull::from(&property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(format.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status("read process tap format", status)?;
    // SAFETY: the successful property call initialized the ASBD.
    let format = unsafe { format.assume_init() };
    validate_stream_format(&format)
}

fn validate_stream_format(format: &AudioStreamBasicDescription) -> Result<(), PlatformError> {
    let non_interleaved = format.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;
    let expected_bytes_per_frame = if non_interleaved {
        size_of::<f32>() as u32
    } else {
        size_of::<f32>() as u32 * format.mChannelsPerFrame
    };
    let supported = format.mFormatID == kAudioFormatLinearPCM
        && format.mFormatFlags & kAudioFormatFlagIsFloat != 0
        && format.mBitsPerChannel == 32
        && format.mChannelsPerFrame > 0
        && format.mBytesPerFrame == expected_bytes_per_frame;
    if supported {
        Ok(())
    } else {
        Err(PlatformError::new(format!(
            "unsupported process tap format (format={}, flags={:#x}, bits={}, channels={}, bytes_per_frame={})",
            format.mFormatID,
            format.mFormatFlags,
            format.mBitsPerChannel,
            format.mChannelsPerFrame,
            format.mBytesPerFrame
        )))
    }
}

fn destroy_tap(tap_id: AudioObjectID) -> Result<(), PlatformError> {
    // SAFETY: tap_id was returned by AudioHardwareCreateProcessTap.
    let status = unsafe { AudioHardwareDestroyProcessTap(tap_id) };
    check_status("destroy process tap", status)
}

fn destroy_aggregate_and_tap(
    aggregate_id: AudioObjectID,
    tap_id: AudioObjectID,
) -> Result<(), PlatformError> {
    // SAFETY: aggregate_id was returned by AudioHardwareCreateAggregateDevice.
    let aggregate_status = unsafe { AudioHardwareDestroyAggregateDevice(aggregate_id) };
    check_status("destroy aggregate device", aggregate_status)?;
    destroy_tap(tap_id)
}

fn record_status(errors: &mut Vec<PlatformError>, operation: &str, status: i32) {
    if status != 0 {
        errors.push(crate::status_error(operation, status));
    }
}

fn errors_to_result(errors: Vec<PlatformError>) -> Result<(), PlatformError> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(PlatformError::new(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }
}

fn with_cleanup_error(primary: PlatformError, cleanup: Result<(), PlatformError>) -> PlatformError {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => PlatformError::new(format!("{primary}; cleanup also failed: {cleanup}")),
    }
}

fn create_aggregate(
    output_uid: &NSString,
    tap_uid: &NSString,
) -> Result<AudioObjectID, PlatformError> {
    let subdevice = ns_dictionary(&[kAudioSubDeviceUIDKey], &[output_uid.as_ref()]);
    let subdevices = NSArray::<NSObject>::from_retained_slice(&[Retained::into_super(subdevice)]);

    let drift = NSNumber::numberWithBool(true);
    let quality = NSNumber::numberWithUnsignedInt(kAudioAggregateDriftCompensationMaxQuality);
    let sub_tap = ns_dictionary(
        &[
            kAudioSubTapUIDKey,
            kAudioSubTapDriftCompensationKey,
            kAudioSubTapDriftCompensationQualityKey,
        ],
        &[tap_uid.as_ref(), drift.as_ref(), quality.as_ref()],
    );
    let taps = NSArray::<NSObject>::from_retained_slice(&[Retained::into_super(sub_tap)]);

    let name = NSString::from_str("Tunic private aggregate");
    let uid = NSUUID::new().UUIDString();
    let private = NSNumber::numberWithBool(true);
    let stacked = NSNumber::numberWithBool(false);
    let auto_start = NSNumber::numberWithBool(true);
    let aggregate = ns_dictionary(
        &[
            kAudioAggregateDeviceNameKey,
            kAudioAggregateDeviceUIDKey,
            kAudioAggregateDeviceIsPrivateKey,
            kAudioAggregateDeviceIsStackedKey,
            kAudioAggregateDeviceTapAutoStartKey,
            kAudioAggregateDeviceMainSubDeviceKey,
            kAudioAggregateDeviceSubDeviceListKey,
            kAudioAggregateDeviceTapListKey,
        ],
        &[
            name.as_ref(),
            uid.as_ref(),
            private.as_ref(),
            stacked.as_ref(),
            auto_start.as_ref(),
            output_uid.as_ref(),
            subdevices.as_ref(),
            taps.as_ref(),
        ],
    );
    // SAFETY: NSDictionary and CFDictionary are toll-free bridged.
    let dictionary = unsafe { &*(Retained::as_ptr(&aggregate) as *const CFDictionary) };
    let mut aggregate_id = 0;
    let status =
        unsafe { AudioHardwareCreateAggregateDevice(dictionary, NonNull::from(&mut aggregate_id)) };
    check_status("create private aggregate device", status)?;
    Ok(aggregate_id)
}

fn ns_dictionary(
    keys: &[&CStr],
    values: &[&NSObject],
) -> Retained<NSDictionary<NSString, NSObject>> {
    let keys = keys.iter().map(|key| ns_key(key)).collect::<Vec<_>>();
    let key_refs = keys.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    NSDictionary::from_slices::<NSString>(&key_refs, values)
}

fn ns_key(key: &CStr) -> Retained<NSString> {
    NSString::from_str(key.to_str().expect("Core Audio dictionary key is UTF-8"))
}

fn wait_until_alive(device: AudioObjectID) -> Result<(), PlatformError> {
    let property = address(
        kAudioDevicePropertyDeviceIsAlive,
        kAudioObjectPropertyScopeGlobal,
    );
    for _ in 0..30 {
        let mut alive = 0_u32;
        let mut size = size_of::<u32>() as u32;
        // SAFETY: alive is valid output storage for the property.
        let status = unsafe {
            AudioObjectGetPropertyData(
                device,
                NonNull::from(&property),
                0,
                std::ptr::null(),
                NonNull::from(&mut size),
                NonNull::new_unchecked((&raw mut alive).cast::<c_void>()),
            )
        };
        if status == 0 && alive != 0 {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(PlatformError::new(
        "private aggregate device did not become ready",
    ))
}

unsafe fn zero_output(list: *mut AudioBufferList) {
    if list.is_null() {
        return;
    }
    // SAFETY: list is supplied by Core Audio and contains mNumberBuffers entries.
    let buffers = unsafe {
        std::slice::from_raw_parts_mut(
            (*list).mBuffers.as_mut_ptr(),
            (*list).mNumberBuffers as usize,
        )
    };
    for buffer in buffers {
        if !buffer.mData.is_null() {
            // SAFETY: mData points to mDataByteSize writable bytes.
            unsafe { std::ptr::write_bytes(buffer.mData, 0, buffer.mDataByteSize as usize) };
        }
    }
}

fn render_audio(
    input: *mut AudioBufferList,
    output: *mut AudioBufferList,
    tap_buffers: TapBufferRange,
    scratch: &mut [f32],
    graph: &mut PreparedGraph,
    bypassed: bool,
    processed_output: &mut ProcessedOutputObservers,
) {
    if input.is_null() || output.is_null() {
        return;
    }
    // SAFETY: both lists are supplied by Core Audio for this callback.
    let input_buffers = unsafe {
        std::slice::from_raw_parts((*input).mBuffers.as_ptr(), (*input).mNumberBuffers as usize)
    };
    let output_buffers = unsafe {
        std::slice::from_raw_parts_mut(
            (*output).mBuffers.as_mut_ptr(),
            (*output).mNumberBuffers as usize,
        )
    };
    let Some(input_buffers) = tap_buffers.select(input_buffers) else {
        return;
    };
    let capacity = writable_frames(output_buffers).min(scratch.len() / 2);
    let frames = normalize_stereo(input_buffers, scratch, capacity);
    if frames == 0 {
        return;
    }
    if !bypassed {
        graph.process_interleaved_stereo(&mut scratch[..frames * 2]);
    }
    if write_stereo(&scratch[..frames * 2], output_buffers, frames) {
        processed_output.observe(&scratch[..frames * 2]);
    }
}

struct ProcessedOutputObservers {
    output_sink: Option<Arc<dyn ProcessedOutputSink>>,
    telemetry: TelemetryPublisher,
    telemetry_generation: Option<TelemetryGeneration>,
    level_meter: LevelMeter,
    spectrum_meter: SpectrumMeter,
}

impl ProcessedOutputObservers {
    fn new(
        output_sink: Option<Arc<dyn ProcessedOutputSink>>,
        telemetry: TelemetryPublisher,
        sample_rate_hz: f64,
    ) -> Self {
        Self {
            output_sink,
            telemetry,
            telemetry_generation: None,
            level_meter: LevelMeter::new(sample_rate_hz),
            spectrum_meter: SpectrumMeter::new(sample_rate_hz),
        }
    }

    fn observe(&mut self, samples: &[f32]) {
        if let Some(generation) = self.telemetry.active_generation() {
            if self.telemetry_generation != Some(generation) {
                self.level_meter.reset();
                self.spectrum_meter.reset();
                self.telemetry_generation = Some(generation);
            }
            self.spectrum_meter.observe(samples);
            if let Some(levels) = self.level_meter.observe(samples)
                && self.spectrum_meter.is_ready()
            {
                self.telemetry.publish(
                    generation,
                    TelemetryFrame {
                        levels,
                        spectrum: self.spectrum_meter.current(),
                    },
                );
            }
        } else {
            self.telemetry_generation = None;
        }
        if let Some(output_sink) = &self.output_sink {
            output_sink.write(samples);
        }
    }
}

struct LevelMeter {
    window_frames: usize,
    frames: usize,
    left_peak: f32,
    right_peak: f32,
    left_square_sum: f64,
    right_square_sum: f64,
    smoothed: StereoLevels,
    peak_decay: f32,
    rms_decay: f32,
}

impl LevelMeter {
    fn new(sample_rate_hz: f64) -> Self {
        let window_frames = (sample_rate_hz / TELEMETRY_UPDATES_PER_SECOND).round() as usize;
        Self::with_window_frames_and_rate(window_frames, sample_rate_hz)
    }

    #[cfg(test)]
    fn with_window_frames(window_frames: usize) -> Self {
        Self::with_window_frames_and_rate(
            window_frames,
            window_frames as f64 * TELEMETRY_UPDATES_PER_SECOND,
        )
    }

    fn with_window_frames_and_rate(window_frames: usize, sample_rate_hz: f64) -> Self {
        let update_rate = sample_rate_hz / window_frames.max(1) as f64;
        Self {
            window_frames: window_frames.max(1),
            frames: 0,
            left_peak: 0.0,
            right_peak: 0.0,
            left_square_sum: 0.0,
            right_square_sum: 0.0,
            smoothed: StereoLevels::default(),
            peak_decay: decay_multiplier(PEAK_DECAY_DB_PER_SECOND, update_rate),
            rms_decay: decay_multiplier(RMS_DECAY_DB_PER_SECOND, update_rate),
        }
    }

    fn observe(&mut self, interleaved_stereo: &[f32]) -> Option<StereoLevels> {
        let mut latest = None;
        for frame in interleaved_stereo.as_chunks::<2>().0 {
            let left = frame[0];
            let right = frame[1];
            self.left_peak = self.left_peak.max(left.abs());
            self.right_peak = self.right_peak.max(right.abs());
            self.left_square_sum += f64::from(left) * f64::from(left);
            self.right_square_sum += f64::from(right) * f64::from(right);
            self.frames += 1;

            if self.frames == self.window_frames {
                let frames = self.frames as f64;
                let measured = StereoLevels {
                    left: ChannelLevels {
                        peak: self.left_peak,
                        rms: (self.left_square_sum / frames).sqrt() as f32,
                    },
                    right: ChannelLevels {
                        peak: self.right_peak,
                        rms: (self.right_square_sum / frames).sqrt() as f32,
                    },
                };
                self.smoothed.left = smooth_levels(
                    self.smoothed.left,
                    measured.left,
                    self.peak_decay,
                    self.rms_decay,
                );
                self.smoothed.right = smooth_levels(
                    self.smoothed.right,
                    measured.right,
                    self.peak_decay,
                    self.rms_decay,
                );
                latest = Some(self.smoothed);
                self.frames = 0;
                self.left_peak = 0.0;
                self.right_peak = 0.0;
                self.left_square_sum = 0.0;
                self.right_square_sum = 0.0;
            }
        }
        latest
    }

    fn reset(&mut self) {
        self.frames = 0;
        self.left_peak = 0.0;
        self.right_peak = 0.0;
        self.left_square_sum = 0.0;
        self.right_square_sum = 0.0;
        self.smoothed = StereoLevels::default();
    }
}

fn smooth_levels(
    current: ChannelLevels,
    measured: ChannelLevels,
    peak_decay: f32,
    rms_decay: f32,
) -> ChannelLevels {
    ChannelLevels {
        peak: smooth_with_decay(current.peak, measured.peak, 1.0, peak_decay),
        rms: smooth_with_decay(current.rms, measured.rms, 0.35, rms_decay),
    }
}

fn smooth_with_decay(current: f32, measured: f32, attack: f32, decay: f32) -> f32 {
    if current == 0.0 {
        measured
    } else if measured >= current {
        current + (measured - current) * attack
    } else {
        measured.max(current * decay)
    }
}

fn decay_multiplier(decibels_per_second: f32, updates_per_second: f64) -> f32 {
    10.0_f32.powf(-decibels_per_second / (20.0 * updates_per_second as f32))
}

struct SpectrumMeter {
    hop_frames: usize,
    frames_since_analysis: usize,
    filled: usize,
    write_index: usize,
    left: Vec<f32>,
    right: Vec<f32>,
    window: Vec<f32>,
    fft: SpectrumFft,
    smoothed: Spectrum,
    decay: f32,
    ready: bool,
}

impl SpectrumMeter {
    fn new(sample_rate_hz: f64) -> Self {
        let fft_size = spectrum_fft_size(sample_rate_hz);
        let mut window = vec![0.0; fft_size];
        for (index, value) in window.iter_mut().enumerate() {
            *value = 0.5 - 0.5 * (TAU * index as f32 / (fft_size - 1) as f32).cos();
        }
        Self {
            hop_frames: (sample_rate_hz / TELEMETRY_UPDATES_PER_SECOND)
                .round()
                .max(1.0) as usize,
            frames_since_analysis: 0,
            filled: 0,
            write_index: 0,
            left: vec![0.0; fft_size],
            right: vec![0.0; fft_size],
            window,
            fft: SpectrumFft::new(fft_size, sample_rate_hz),
            smoothed: Spectrum::default(),
            decay: decay_multiplier(SPECTRUM_DECAY_DB_PER_SECOND, TELEMETRY_UPDATES_PER_SECOND),
            ready: false,
        }
    }

    fn observe(&mut self, interleaved_stereo: &[f32]) {
        let fft_size = self.left.len();
        for frame in interleaved_stereo.as_chunks::<2>().0 {
            self.left[self.write_index] = frame[0];
            self.right[self.write_index] = frame[1];
            self.write_index = (self.write_index + 1) % fft_size;
            self.filled = (self.filled + 1).min(fft_size);
            self.frames_since_analysis += 1;

            if self.filled == fft_size && self.frames_since_analysis >= self.hop_frames {
                self.analyze();
                self.frames_since_analysis = 0;
            }
        }
    }

    fn current(&self) -> Spectrum {
        self.smoothed
    }

    fn is_ready(&self) -> bool {
        self.ready
    }

    fn reset(&mut self) {
        self.frames_since_analysis = 0;
        self.filled = 0;
        self.write_index = 0;
        self.smoothed = Spectrum::default();
        self.ready = false;
    }

    fn analyze(&mut self) {
        let mut measured = [0.0; SPECTRUM_BAND_COUNT];
        self.fft
            .analyze(&self.left, self.write_index, &self.window, &mut measured);
        self.fft
            .analyze(&self.right, self.write_index, &self.window, &mut measured);
        for (smoothed, measured) in self.smoothed.bands.iter_mut().zip(measured) {
            *smoothed = smooth_with_decay(*smoothed, measured, SPECTRUM_ATTACK, self.decay);
        }
        self.ready = true;
    }
}

fn spectrum_fft_size(sample_rate_hz: f64) -> usize {
    ((sample_rate_hz / SPECTRUM_MAX_BIN_WIDTH_HZ).ceil() as usize)
        .max(2)
        .next_power_of_two()
}

struct SpectrumFft {
    plan: Arc<dyn Fft<f32>>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    band_bins: [(usize, usize); SPECTRUM_BAND_COUNT],
}

impl SpectrumFft {
    fn new(fft_size: usize, sample_rate_hz: f64) -> Self {
        let plan = FftPlanner::new().plan_fft_forward(fft_size);
        let scratch = vec![Complex32::default(); plan.get_inplace_scratch_len()];
        Self {
            plan,
            buffer: vec![Complex32::default(); fft_size],
            scratch,
            band_bins: spectrum_band_bins(fft_size, sample_rate_hz),
        }
    }

    fn analyze(
        &mut self,
        samples: &[f32],
        start: usize,
        window: &[f32],
        bands: &mut [f32; SPECTRUM_BAND_COUNT],
    ) {
        let fft_size = self.buffer.len();
        for index in 0..fft_size {
            self.buffer[index] =
                Complex32::new(samples[(start + index) % fft_size] * window[index], 0.0);
        }
        self.plan
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        let amplitude_scale = 4.0 / fft_size as f32;
        for (band, &(first, last)) in bands.iter_mut().zip(&self.band_bins) {
            if first <= last {
                for value in &self.buffer[first..=last] {
                    let amplitude = value.norm() * amplitude_scale;
                    *band = band.max(amplitude);
                }
            }
        }
    }
}

fn spectrum_band_bins(
    fft_size: usize,
    sample_rate_hz: f64,
) -> [(usize, usize); SPECTRUM_BAND_COUNT] {
    let bin_width_hz = sample_rate_hz / fft_size as f64;
    std::array::from_fn(|index| {
        let center = f64::from(SPECTRUM_FREQUENCIES_HZ[index]);
        let lower_hz = if index == 0 {
            let next = f64::from(SPECTRUM_FREQUENCIES_HZ[1]);
            center / (next / center).sqrt()
        } else {
            let previous = f64::from(SPECTRUM_FREQUENCIES_HZ[index - 1]);
            (previous * center).sqrt()
        };
        let upper_hz = if index + 1 == SPECTRUM_BAND_COUNT {
            let previous = f64::from(SPECTRUM_FREQUENCIES_HZ[index - 1]);
            center * (center / previous).sqrt()
        } else {
            let next = f64::from(SPECTRUM_FREQUENCIES_HZ[index + 1]);
            (center * next).sqrt()
        };
        let first = (lower_hz / bin_width_hz).ceil() as usize;
        let last = ((upper_hz / bin_width_hz).ceil() as usize)
            .saturating_sub(1)
            .min(fft_size / 2);
        (first.max(1), last)
    })
}

fn writable_frames(buffers: &[AudioBuffer]) -> usize {
    buffers
        .iter()
        .filter(|buffer| !buffer.mData.is_null())
        .map(|buffer| {
            buffer.mDataByteSize as usize
                / (size_of::<f32>() * (buffer.mNumberChannels as usize).max(1))
        })
        .min()
        .unwrap_or(0)
}

fn normalize_stereo(buffers: &[AudioBuffer], output: &mut [f32], limit: usize) -> usize {
    let Some(first) = buffers.first() else {
        return 0;
    };
    if buffers.len() >= 2
        && first.mNumberChannels == 1
        && buffers[1].mNumberChannels == 1
        && !first.mData.is_null()
        && !buffers[1].mData.is_null()
    {
        let frames = limit
            .min(first.mDataByteSize as usize / size_of::<f32>())
            .min(buffers[1].mDataByteSize as usize / size_of::<f32>());
        // SAFETY: each source contains at least frames f32 samples.
        let left = unsafe { std::slice::from_raw_parts(first.mData.cast::<f32>(), frames) };
        let right = unsafe { std::slice::from_raw_parts(buffers[1].mData.cast::<f32>(), frames) };
        for frame in 0..frames {
            output[frame * 2] = left[frame];
            output[frame * 2 + 1] = right[frame];
        }
        return frames;
    }
    if first.mData.is_null() {
        return 0;
    }
    let channels = (first.mNumberChannels as usize).max(1);
    let frames = limit.min(first.mDataByteSize as usize / (size_of::<f32>() * channels));
    // SAFETY: source contains frames * channels f32 samples.
    let source =
        unsafe { std::slice::from_raw_parts(first.mData.cast::<f32>(), frames * channels) };
    for frame in 0..frames {
        output[frame * 2] = source[frame * channels];
        output[frame * 2 + 1] = source[frame * channels + usize::from(channels > 1)];
    }
    frames
}

fn write_stereo(samples: &[f32], buffers: &mut [AudioBuffer], frames: usize) -> bool {
    let Some(first) = buffers.first_mut() else {
        return false;
    };
    if first.mNumberChannels >= 2 && !first.mData.is_null() {
        let channels = first.mNumberChannels as usize;
        // SAFETY: writable_frames limited frames to this buffer's capacity.
        let destination =
            unsafe { std::slice::from_raw_parts_mut(first.mData.cast::<f32>(), frames * channels) };
        for frame in 0..frames {
            destination[frame * channels] = samples[frame * 2];
            destination[frame * channels + 1] = samples[frame * 2 + 1];
        }
        return true;
    }
    if buffers.len() < 2
        || buffers[0].mNumberChannels == 0
        || buffers[1].mNumberChannels == 0
        || buffers[0].mData.is_null()
        || buffers[1].mData.is_null()
    {
        return false;
    }
    for (channel, buffer) in buffers.iter_mut().take(2).enumerate() {
        // SAFETY: writable_frames limited frames to this buffer's capacity.
        let destination =
            unsafe { std::slice::from_raw_parts_mut(buffer.mData.cast::<f32>(), frames) };
        for frame in 0..frames {
            destination[frame] = samples[frame * 2 + channel];
        }
    }
    true
}

const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::{
        GraphExchange, LevelMeter, SpectrumMeter, TapBufferRange, normalize_stereo,
        reset_graph_on_bypass, validate_stream_format, write_stereo,
    };
    use objc2_core_audio_types::{
        AudioBuffer, AudioStreamBasicDescription, kAudioFormatFlagIsFloat,
        kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
    };
    use std::cell::Cell;
    use tunic_dsp::{Equalizer, Filter, FrequencyHz, GainDb, PreparedGraph, QualityFactor};
    use tunic_engine::{ChannelLevels, Spectrum, StereoLevels};

    #[test]
    fn level_meter_publishes_asymmetric_peak_and_rms_windows() {
        let mut meter = LevelMeter::with_window_frames(2);

        let levels = meter.observe(&[1.0, -0.5, 0.0, -0.5]);

        assert_eq!(
            levels,
            Some(StereoLevels {
                left: ChannelLevels {
                    peak: 1.0,
                    rms: std::f32::consts::FRAC_1_SQRT_2,
                },
                right: ChannelLevels {
                    peak: 0.5,
                    rms: 0.5,
                },
            })
        );
    }

    #[test]
    fn level_meter_decays_instead_of_dropping_to_silence() {
        let mut meter = LevelMeter::with_window_frames(2);
        let initial = meter.observe(&[1.0, 0.5, -1.0, -0.5]).unwrap();

        let decayed = meter.observe(&[0.0; 4]).unwrap();

        assert!(decayed.left.peak < initial.left.peak);
        assert!(decayed.left.peak > 0.9);
        assert!(decayed.left.rms < initial.left.rms);
        assert!(decayed.left.rms > 0.9);
    }

    #[test]
    fn spectrum_meter_separates_asymmetric_tones_and_decays() {
        const SAMPLE_RATE: f32 = 48_000.0;
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let fft_size = meter.left.len();
        let samples = (0..fft_size)
            .flat_map(|frame| {
                let time = frame as f32 / SAMPLE_RATE;
                [
                    0.5 * (std::f32::consts::TAU * 1_000.0 * time).sin(),
                    0.5 * (std::f32::consts::TAU * 8_000.0 * time).sin(),
                ]
            })
            .collect::<Vec<_>>();

        meter.observe(&samples);
        let measured = meter.current();

        assert!(meter.is_ready());
        assert!(measured.bands[15] > 0.2, "1 kHz: {}", measured.bands[15]);
        assert!(measured.bands[24] > 0.2, "8 kHz: {}", measured.bands[24]);
        assert!(measured.bands[11] < 0.01, "400 Hz: {}", measured.bands[11]);

        meter.left.fill(0.0);
        meter.right.fill(0.0);
        meter.analyze();
        let decayed = meter.current();
        assert!(decayed.bands[15] < measured.bands[15]);
        assert!(decayed.bands[15] > 0.0);

        meter.reset();
        assert!(!meter.is_ready());
        assert_eq!(meter.current(), Spectrum::default());
    }

    #[test]
    fn spectrum_bands_are_distinct_and_contiguous_at_supported_sample_rates() {
        for sample_rate_hz in [48_000.0, 192_000.0] {
            let meter = SpectrumMeter::new(sample_rate_hz);
            let bins = meter.fft.band_bins;

            assert_ne!(bins[0], bins[1]);
            for adjacent in bins.windows(2) {
                assert_eq!(adjacent[0].1 + 1, adjacent[1].0);
            }
            assert!(sample_rate_hz / meter.left.len() as f64 <= 6.0);
        }
    }

    #[test]
    fn spectrum_assigns_tones_between_rounded_centers_to_a_band() {
        const SAMPLE_RATE: f32 = 48_000.0;
        const TONE_HZ: f32 = 14_144.531;
        let mut meter = SpectrumMeter::new(f64::from(SAMPLE_RATE));
        let fft_size = meter.left.len();
        let samples = (0..fft_size)
            .flat_map(|frame| {
                let sample =
                    0.5 * (std::f32::consts::TAU * TONE_HZ * frame as f32 / SAMPLE_RATE).sin();
                [sample, sample]
            })
            .collect::<Vec<_>>();

        meter.observe(&samples);

        assert!(meter.current().bands[27] > 0.4);
    }

    #[test]
    fn graph_exchange_installs_the_latest_pending_graph() {
        let exchange = GraphExchange::new();
        exchange.publish(peaking_graph(6.0));
        exchange.publish(peaking_graph(-6.0));
        let mut current = PreparedGraph::identity();
        let mut impulse = [1.0_f32, 1.0];

        exchange.install_latest(&mut current);
        current.process_interleaved_stereo(&mut impulse);

        assert!(impulse[0] < 1.0);
        assert_eq!(impulse[0], impulse[1]);
    }

    #[test]
    fn entering_bypass_clears_filter_history() {
        let mut graph = peaking_graph(12.0);
        let mut impulse = [1.0_f32, 1.0];
        graph.process_interleaved_stereo(&mut impulse);
        let was_bypassed = Cell::new(false);

        reset_graph_on_bypass(&mut graph, &was_bypassed, true);
        let mut silence = [0.0_f32, 0.0];
        graph.process_interleaved_stereo(&mut silence);

        assert_eq!(silence, [0.0, 0.0]);
    }

    #[test]
    fn normalizes_asymmetric_planar_input_to_interleaved_stereo() {
        let mut left = [0.25_f32, -0.75];
        let mut right = [-0.5_f32, 0.125];
        let buffers = [mono_buffer(&mut left), mono_buffer(&mut right)];
        let mut output = [0.0; 4];

        let frames = normalize_stereo(&buffers, &mut output, 2);

        assert_eq!(frames, 2);
        assert_eq!(output, [0.25, -0.5, -0.75, 0.125]);
    }

    #[test]
    fn writes_stereo_without_overwriting_extra_output_channels() {
        let input = [0.25_f32, -0.5, -0.75, 0.125];
        let mut destination = [9.0_f32; 8];
        let mut buffers = [AudioBuffer {
            mNumberChannels: 4,
            mDataByteSize: size_of_val(&destination) as u32,
            mData: destination.as_mut_ptr().cast(),
        }];

        write_stereo(&input, &mut buffers, 2);

        assert_eq!(destination, [0.25, -0.5, 9.0, 9.0, -0.75, 0.125, 9.0, 9.0]);
    }

    #[test]
    fn tap_range_excludes_physical_input_buffers() {
        let mut microphone = [0.9_f32, 0.8];
        let mut tap_left = [0.25_f32, -0.75];
        let mut tap_right = [-0.5_f32, 0.125];
        let buffers = [
            mono_buffer(&mut microphone),
            mono_buffer(&mut tap_left),
            mono_buffer(&mut tap_right),
        ];
        let range = TapBufferRange::from_stream_counts(3, 1).unwrap();
        let mut output = [0.0; 4];

        let frames = normalize_stereo(range.select(&buffers).unwrap(), &mut output, 2);

        assert_eq!(frames, 2);
        assert_eq!(output, [0.25, -0.5, -0.75, 0.125]);
    }

    #[test]
    fn tap_range_rejects_missing_or_changed_layouts() {
        assert!(TapBufferRange::from_stream_counts(2, 2).is_err());
        assert!(TapBufferRange::from_stream_counts(1, 2).is_err());

        let range = TapBufferRange::from_stream_counts(3, 1).unwrap();
        let mut only_one_buffer = [0.0_f32; 2];
        assert!(range.select(&[mono_buffer(&mut only_one_buffer)]).is_none());
    }

    #[test]
    fn accepts_supported_float32_tap_formats() {
        assert!(validate_stream_format(&float32_format(2, 8, 0)).is_ok());
        assert!(
            validate_stream_format(&float32_format(2, 4, kAudioFormatFlagIsNonInterleaved,))
                .is_ok()
        );
    }

    #[test]
    fn rejects_non_float_and_misaligned_tap_formats() {
        let mut integer = float32_format(2, 8, 0);
        integer.mFormatFlags = 0;
        assert!(validate_stream_format(&integer).is_err());

        let misaligned = float32_format(2, 4, 0);
        assert!(validate_stream_format(&misaligned).is_err());
    }

    fn mono_buffer(samples: &mut [f32]) -> AudioBuffer {
        AudioBuffer {
            mNumberChannels: 1,
            mDataByteSize: size_of_val(samples) as u32,
            mData: samples.as_mut_ptr().cast(),
        }
    }

    fn float32_format(
        channels: u32,
        bytes_per_frame: u32,
        additional_flags: u32,
    ) -> AudioStreamBasicDescription {
        AudioStreamBasicDescription {
            mSampleRate: 48_000.0,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | additional_flags,
            mBytesPerPacket: bytes_per_frame,
            mFramesPerPacket: 1,
            mBytesPerFrame: bytes_per_frame,
            mChannelsPerFrame: channels,
            mBitsPerChannel: 32,
            mReserved: 0,
        }
    }

    fn peaking_graph(gain_db: f64) -> PreparedGraph {
        let equalizer = Equalizer::with_filter(Filter::peaking(
            FrequencyHz::new(1_000.0).unwrap(),
            GainDb::new(gain_db).unwrap(),
            QualityFactor::new(1.0).unwrap(),
        ));
        PreparedGraph::prepare(&equalizer, 48_000.0).unwrap()
    }
}
