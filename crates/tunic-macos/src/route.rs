use std::cell::RefCell;
use std::ffi::{CStr, c_void};
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
use tunic_dsp::PreparedGraph;
use tunic_engine::PlatformError;

use crate::devices::{device_uid, input_stream_count};
use crate::{address, check_status};

const SCRATCH_FRAME_CAPACITY: usize = 16_384;

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
        bypassed: Arc<AtomicBool>,
    ) -> Result<Self, PlatformError> {
        Self::start_inner(output_id, bypassed)
    }

    fn start_inner(
        output_id: AudioObjectID,
        bypassed: Arc<AtomicBool>,
    ) -> Result<Self, PlatformError> {
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
        let graph = RefCell::new(PreparedGraph::identity());
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
                    let (Ok(mut scratch), Ok(mut graph)) =
                        (scratch.try_borrow_mut(), graph.try_borrow_mut())
                    else {
                        return;
                    };
                    render_identity(
                        input.as_ptr(),
                        output.as_ptr(),
                        tap_buffers,
                        &mut scratch[..],
                        &mut graph,
                        bypassed.load(Ordering::Relaxed),
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

fn render_identity(
    input: *mut AudioBufferList,
    output: *mut AudioBufferList,
    tap_buffers: TapBufferRange,
    scratch: &mut [f32],
    graph: &mut PreparedGraph,
    bypassed: bool,
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
    write_stereo(&scratch[..frames * 2], output_buffers, frames);
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

fn write_stereo(samples: &[f32], buffers: &mut [AudioBuffer], frames: usize) {
    let Some(first) = buffers.first_mut() else {
        return;
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
        return;
    }
    for (channel, buffer) in buffers.iter_mut().take(2).enumerate() {
        if buffer.mData.is_null() {
            continue;
        }
        // SAFETY: writable_frames limited frames to this buffer's capacity.
        let destination =
            unsafe { std::slice::from_raw_parts_mut(buffer.mData.cast::<f32>(), frames) };
        for frame in 0..frames {
            destination[frame] = samples[frame * 2 + channel];
        }
    }
}

const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    use super::{TapBufferRange, normalize_stereo, validate_stream_format, write_stereo};
    use objc2_core_audio_types::{
        AudioBuffer, AudioStreamBasicDescription, kAudioFormatFlagIsFloat,
        kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
    };

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
}
