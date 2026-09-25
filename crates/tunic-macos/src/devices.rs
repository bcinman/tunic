use std::ffi::c_void;
use std::ptr::NonNull;

use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID,
    AudioObjectPropertyAddress, kAudioDevicePropertyDeviceUID,
    kAudioDevicePropertyNominalSampleRate, kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyStreams, kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal,
    kAudioObjectPropertyScopeInput, kAudioObjectPropertyScopeOutput, kAudioObjectSystemObject,
};
use objc2_core_audio_types::AudioBufferList;
use objc2_core_foundation::{CFRetained, CFString};
use tunic_engine::{DeviceId, OutputDevice, PlatformError};

use crate::{address, check_status};

pub(crate) fn list_output_devices() -> Result<Vec<OutputDevice>, PlatformError> {
    let default = default_output_id()?;
    let mut devices = Vec::new();

    for id in all_device_ids()? {
        if !is_output_device(id) {
            continue;
        }
        let Some(uid) = string_property(id, kAudioDevicePropertyDeviceUID) else {
            continue;
        };
        let Some(name) = string_property(id, kAudioObjectPropertyName) else {
            continue;
        };
        devices.push(OutputDevice {
            id: DeviceId::new(uid),
            name,
            sample_rate_hz: sample_rate(id).unwrap_or(0.0),
            channels: channel_count(id).unwrap_or(0),
            is_default: id == default,
        });
    }

    Ok(devices)
}

pub(crate) fn default_output_id() -> Result<AudioObjectID, PlatformError> {
    let property = address(
        kAudioHardwarePropertyDefaultOutputDevice,
        kAudioObjectPropertyScopeGlobal,
    );
    read_property(kAudioObjectSystemObject as AudioObjectID, &property)
}

pub(crate) fn device_uid(id: AudioObjectID) -> Result<String, PlatformError> {
    string_property(id, kAudioDevicePropertyDeviceUID)
        .ok_or_else(|| PlatformError::new(format!("output device {id} has no UID")))
}

pub(crate) fn device_name(id: AudioObjectID) -> Result<String, PlatformError> {
    string_property(id, kAudioObjectPropertyName)
        .ok_or_else(|| PlatformError::new(format!("output device {id} has no name")))
}

pub(crate) fn sample_rate(id: AudioObjectID) -> Result<f64, PlatformError> {
    let property = address(
        kAudioDevicePropertyNominalSampleRate,
        kAudioObjectPropertyScopeOutput,
    );
    read_property(id, &property)
}

pub(crate) fn channel_count(id: AudioObjectID) -> Result<u32, PlatformError> {
    let property = address(
        kAudioDevicePropertyStreamConfiguration,
        kAudioObjectPropertyScopeOutput,
    );
    let mut size = property_size(id, &property)?;
    let words = (size as usize)
        .div_ceil(size_of::<AudioBufferList>())
        .max(1);
    let mut storage = Vec::<AudioBufferList>::with_capacity(words);

    // SAFETY: the vector has enough aligned capacity for the byte count Core Audio reported.
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(&property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(storage.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status("read output stream configuration", status)?;

    let list = storage.as_ptr();
    // SAFETY: Core Audio initialized an AudioBufferList in the aligned allocation above.
    let buffers = unsafe {
        std::slice::from_raw_parts((*list).mBuffers.as_ptr(), (*list).mNumberBuffers as usize)
    };
    Ok(buffers.iter().map(|buffer| buffer.mNumberChannels).sum())
}

pub(crate) fn input_stream_count(id: AudioObjectID) -> Result<usize, PlatformError> {
    let property = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    property_size(id, &property).map(|size| size as usize / size_of::<AudioObjectID>())
}

fn all_device_ids() -> Result<Vec<AudioObjectID>, PlatformError> {
    let property = address(
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
    );
    let mut size = property_size(kAudioObjectSystemObject as AudioObjectID, &property)?;
    let count = size as usize / size_of::<AudioObjectID>();
    let mut ids = vec![0; count];

    // SAFETY: ids has exactly the capacity reported by Core Audio.
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&property),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(ids.as_mut_ptr().cast::<c_void>()),
        )
    };
    check_status("list audio devices", status)?;
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    Ok(ids)
}

fn is_output_device(id: AudioObjectID) -> bool {
    let property = address(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeOutput);
    property_size(id, &property).is_ok_and(|size| size > 0)
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

fn property_size(
    id: AudioObjectID,
    property: &AudioObjectPropertyAddress,
) -> Result<u32, PlatformError> {
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
) -> Result<T, PlatformError> {
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
