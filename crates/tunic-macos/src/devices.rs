use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::Arc;

use block2::RcBlock;
use objc2_core_audio::{
    AudioObjectAddPropertyListenerBlock, AudioObjectGetPropertyData,
    AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    AudioObjectRemovePropertyListenerBlock, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreams,
    kAudioHardwarePropertyDefaultOutputDevice, kAudioObjectPropertyName,
    kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyScopeInput,
    kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject, kAudioStreamPropertyVirtualFormat,
};
use objc2_core_audio_types::AudioStreamBasicDescription;
use objc2_core_foundation::{CFRetained, CFString};

use crate::{Error, address, check_status};

type ListenerBlock = RcBlock<dyn Fn(u32, NonNull<AudioObjectPropertyAddress>)>;

/// Owns a listener for changes to the system's default output.
pub(super) struct DefaultOutputWatcher {
    property: AudioObjectPropertyAddress,
    block: ListenerBlock,
}

impl DefaultOutputWatcher {
    pub(super) fn start(notify: Arc<dyn Fn() + Send + Sync + 'static>) -> Result<Self, Error> {
        let property = address(
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioObjectPropertyScopeGlobal,
        );
        let block: ListenerBlock = RcBlock::new(
            move |_count: u32, _addresses: NonNull<AudioObjectPropertyAddress>| {
                let _ = catch_unwind(AssertUnwindSafe(|| notify()));
            },
        );
        // SAFETY: property and block remain owned by the watcher until it unregisters them.
        let status = unsafe {
            AudioObjectAddPropertyListenerBlock(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&property),
                None,
                RcBlock::as_ptr(&block),
            )
        };
        check_status("watch default output", status)?;
        Ok(Self { property, block })
    }
}

impl Drop for DefaultOutputWatcher {
    fn drop(&mut self) {
        // SAFETY: this is the same object, property, queue, and block used to register.
        let _ = unsafe {
            AudioObjectRemovePropertyListenerBlock(
                kAudioObjectSystemObject as AudioObjectID,
                NonNull::from(&self.property),
                None,
                RcBlock::as_ptr(&self.block),
            )
        };
    }
}

pub(crate) fn default_output_id() -> Result<AudioObjectID, Error> {
    let property = address(
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyScopeGlobal,
    );
    read_property(kAudioObjectSystemObject as AudioObjectID, &property)
}

pub(crate) fn device_uid(id: AudioObjectID) -> Result<String, Error> {
    string_property(id, kAudioDevicePropertyDeviceUID)
        .ok_or_else(|| Error::new(format!("output device {id} has no UID")))
}

pub(crate) fn device_name(id: AudioObjectID) -> Result<String, Error> {
    string_property(id, kAudioObjectPropertyName)
        .ok_or_else(|| Error::new(format!("output device {id} has no name")))
}

pub(crate) fn sample_rate(id: AudioObjectID) -> Result<f64, Error> {
    let property = address(
        kAudioDevicePropertyNominalSampleRate,
        kAudioObjectPropertyScopeOutput,
    );
    read_property(id, &property)
}

pub(crate) fn input_stream_count(id: AudioObjectID) -> Result<usize, Error> {
    let property = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    property_size(id, &property).map(|size| size as usize / size_of::<AudioObjectID>())
}

/// The IOProc receives virtual stream samples, potentially resampled by the aggregate.
/// Checking the source tap rate alone would not establish the processor's actual rate.
pub(crate) fn validate_callback_sample_rates(
    id: AudioObjectID,
    expected: f64,
) -> Result<(), Error> {
    for scope in [
        kAudioObjectPropertyScopeInput,
        kAudioObjectPropertyScopeOutput,
    ] {
        let property = address(kAudioDevicePropertyStreams, scope);
        let mut bytes = property_size(id, &property)?;
        let mut streams = vec![0; bytes as usize / size_of::<AudioObjectID>()];
        if streams.is_empty() {
            return Err(Error::new("aggregate has no input or output streams"));
        }
        // SAFETY: streams is initialized storage of the size reported by Core Audio.
        let status = unsafe {
            AudioObjectGetPropertyData(
                id,
                NonNull::from(&property),
                0,
                std::ptr::null(),
                NonNull::from(&mut bytes),
                NonNull::new_unchecked(streams.as_mut_ptr().cast::<c_void>()),
            )
        };
        check_status("read aggregate streams", status)?;
        streams.truncate(bytes as usize / size_of::<AudioObjectID>());
        for stream in streams {
            let format: AudioStreamBasicDescription = read_property(
                stream,
                &address(
                    kAudioStreamPropertyVirtualFormat,
                    kAudioObjectPropertyScopeGlobal,
                ),
            )?;
            validate_sample_rate(format.mSampleRate, expected)?;
        }
    }
    Ok(())
}

fn validate_sample_rate(actual: f64, expected: f64) -> Result<(), Error> {
    if actual.is_finite() && actual > 0.0 && actual == expected {
        Ok(())
    } else {
        Err(Error::new(format!(
            "aggregate callback sample rate {actual} does not match DSP rate {expected}"
        )))
    }
}

fn string_property(id: AudioObjectID, selector: u32) -> Option<String> {
    let property = address(selector, kAudioObjectPropertyScopeGlobal);
    let mut value: *const CFString = std::ptr::null();
    let mut size = size_of::<*const CFString>() as u32;

    // SAFETY: value is storage for the retained CFStringRef returned by Core Audio.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(&property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked((&raw mut value).cast::<c_void>()),
        )
    };
    if status != 0 || value.is_null() {
        return None;
    }
    // SAFETY: Core Audio returned a retained CFStringRef.
    let retained = unsafe { CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut())) };
    Some(retained.to_string())
}

fn property_size(id: AudioObjectID, property: &AudioObjectPropertyAddress) -> Result<u32, Error> {
    let mut size = 0;
    // SAFETY: property and size point to valid values for the duration of the call.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            id,
            NonNull::from(property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    check_status("query audio property size", status)?;
    Ok(size)
}

fn read_property<T: Copy>(
    id: AudioObjectID,
    property: &AudioObjectPropertyAddress,
) -> Result<T, Error> {
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut size = size_of::<T>() as u32;
    // SAFETY: value has enough storage for T and Core Audio reports success before it is read.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(value.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status("read audio property", status)?;
    // SAFETY: a successful call initialized value.
    Ok(unsafe { value.assume_init() })
}

const fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
mod tests {
    #[test]
    fn callback_rate_must_match_dsp_rate() {
        for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            assert!(super::validate_sample_rate(rate, rate).is_ok());
            assert!(super::validate_sample_rate(rate, rate + 1.0).is_err());
        }
        for rate in [0.0, -48_000.0, f64::NAN, f64::INFINITY] {
            assert!(super::validate_sample_rate(rate, 48_000.0).is_err());
        }
    }
}
