//! Native bindings for Tunic's portable core.
//!
//! BoltFFI owns control-plane type conversion and object lifetimes. The audio
//! callback uses [`tunic_processor_process_realtime`], whose primitive handle
//! and raw buffer never enter BoltFFI's collection wrappers.

mod backend;
mod presets;
pub use backend::*;
pub use presets::*;

use std::num::NonZeroUsize;
use std::ptr::NonNull;

use boltffi::*;
use tunic_core as core;

pub const PROCESS_OK: u8 = 0;
pub const PROCESS_INVALID_HANDLE: u8 = 1;
pub const PROCESS_INVALID_BUFFER: u8 = 2;
pub const PROCESS_TOO_MANY_FRAMES: u8 = 3;

#[data]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
}

#[data]
pub struct Filter {
    pub id: u32,
    pub kind: FilterKind,
    pub frequency_hz: f64,
    pub gain_db: f64,
    pub quality_factor: f64,
}

#[data]
pub struct Chain {
    pub preamp_gain_db: f64,
    pub filters: Vec<Filter>,
}

#[error]
#[derive(Debug)]
pub enum ProcessorError {
    InvalidSampleRate,
    InvalidMaximumFrameCount,
    InvalidPreampGain,
    InvalidFilterId,
    DuplicateFilterId,
    InvalidFilterFrequency,
    InvalidFilterGain,
    InvalidFilterQualityFactor,
    PreampOutOfRange,
    FilterAtOrAboveNyquist,
    UnstableFilter,
}

/// A processor whose mutation is exclusively owned by one native audio callback.
pub struct Processor {
    inner: core::Processor,
    controller: core::Controller,
    maximum_frame_count: usize,
}

#[export(single_threaded)]
impl Processor {
    pub fn new(
        sample_rate_hz: f64,
        maximum_frame_count: u64,
        chain: Chain,
        bypassed: bool,
    ) -> Result<Self, ProcessorError> {
        let sample_rate = core::SampleRateHz::try_new(sample_rate_hz)
            .map_err(|_| ProcessorError::InvalidSampleRate)?;
        let maximum_frame_count = usize::try_from(maximum_frame_count)
            .ok()
            .and_then(NonZeroUsize::new)
            .ok_or(ProcessorError::InvalidMaximumFrameCount)?;
        let (inner, controller) = core::Processor::new(
            core::AudioFormat {
                sample_rate,
                maximum_frame_count,
            },
            chain.try_into()?,
            bypassed,
        )
        .map_err(ProcessorError::from)?;
        Ok(Self {
            inner,
            controller,
            maximum_frame_count: maximum_frame_count.get(),
        })
    }

    /// Returns the token accepted by [`tunic_processor_process_realtime`].
    ///
    /// The token is valid only while this exact `Processor` remains alive and
    /// no other method or callback accesses it concurrently.
    pub fn realtime_handle(&self) -> u64 {
        std::ptr::from_ref(self).addr() as u64
    }

    pub fn controller(&self) -> Controller {
        Controller {
            inner: self.controller.clone(),
        }
    }
}

/// Cloneable non-real-time control for a [`Processor`].
pub struct Controller {
    inner: core::Controller,
}

#[export]
impl Controller {
    pub fn set_chain(&self, chain: Chain) -> Result<(), ProcessorError> {
        self.inner
            .set_chain(chain.try_into()?)
            .map_err(ProcessorError::from)
    }

    pub fn set_bypassed(&self, bypassed: bool) {
        self.inner.set_bypassed(bypassed);
    }

    pub fn is_bypassed(&self) -> bool {
        self.inner.is_bypassed()
    }
}

/// Processes interleaved stereo in place without allocating, locking, or copying.
///
/// # Safety
///
/// `processor_handle` must come from the live [`Processor::realtime_handle`]
/// instance retained by the native application, and only one callback may use
/// it. `interleaved_stereo` must point to `frame_count * 2` writable `f32`
/// samples, unless `frame_count` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tunic_processor_process_realtime(
    processor_handle: u64,
    interleaved_stereo: *mut f32,
    frame_count: usize,
) -> u8 {
    let Some(mut processor) = NonNull::new(processor_handle as usize as *mut Processor) else {
        return PROCESS_INVALID_HANDLE;
    };
    if frame_count != 0 && (interleaved_stereo.is_null() || !interleaved_stereo.is_aligned()) {
        return PROCESS_INVALID_BUFFER;
    }

    // SAFETY: upheld by the native caller as documented above. The processor
    // object remains pinned inside BoltFFI's class allocation for its lifetime.
    let processor = unsafe { processor.as_mut() };
    if frame_count > processor.maximum_frame_count {
        return PROCESS_TOO_MANY_FRAMES;
    }
    let Some(sample_count) = frame_count.checked_mul(2) else {
        return PROCESS_INVALID_BUFFER;
    };
    let samples = if sample_count == 0 {
        NonNull::dangling().as_ptr()
    } else {
        interleaved_stereo
    };
    // SAFETY: the caller provides writable stereo storage for `frame_count`,
    // validated as non-null and aligned above when it is non-empty.
    let interleaved_stereo = unsafe { std::slice::from_raw_parts_mut(samples, sample_count) };
    processor.inner.process(interleaved_stereo);
    PROCESS_OK
}

impl TryFrom<Chain> for core::Chain {
    type Error = ProcessorError;

    fn try_from(chain: Chain) -> Result<Self, Self::Error> {
        let mut ids = std::collections::HashSet::new();
        if chain.filters.iter().any(|filter| !ids.insert(filter.id)) {
            return Err(ProcessorError::DuplicateFilterId);
        }
        Ok(Self {
            equalizer: core::Equalizer {
                preamp: core::GainDb::try_new(chain.preamp_gain_db)
                    .map_err(|_| ProcessorError::InvalidPreampGain)?,
                filters: chain
                    .filters
                    .into_iter()
                    .map(core::Filter::try_from)
                    .collect::<Result<_, _>>()?,
            },
        })
    }
}

impl TryFrom<Filter> for core::Filter {
    type Error = ProcessorError;

    fn try_from(filter: Filter) -> Result<Self, Self::Error> {
        Ok(Self {
            id: core::FilterId::try_new(filter.id).map_err(|_| ProcessorError::InvalidFilterId)?,
            kind: match filter.kind {
                FilterKind::Peaking => core::FilterKind::Peaking,
                FilterKind::LowShelf => core::FilterKind::LowShelf,
                FilterKind::HighShelf => core::FilterKind::HighShelf,
            },
            frequency: core::FrequencyHz::try_new(filter.frequency_hz)
                .map_err(|_| ProcessorError::InvalidFilterFrequency)?,
            gain: core::GainDb::try_new(filter.gain_db)
                .map_err(|_| ProcessorError::InvalidFilterGain)?,
            quality_factor: core::QualityFactor::try_new(filter.quality_factor)
                .map_err(|_| ProcessorError::InvalidFilterQualityFactor)?,
        })
    }
}

impl From<core::Chain> for Chain {
    fn from(chain: core::Chain) -> Self {
        Self {
            preamp_gain_db: chain.equalizer.preamp.into_inner(),
            filters: chain
                .equalizer
                .filters
                .into_iter()
                .map(|filter| Filter {
                    id: filter.id.into_inner(),
                    kind: match filter.kind {
                        core::FilterKind::Peaking => FilterKind::Peaking,
                        core::FilterKind::LowShelf => FilterKind::LowShelf,
                        core::FilterKind::HighShelf => FilterKind::HighShelf,
                    },
                    frequency_hz: filter.frequency.into_inner(),
                    gain_db: filter.gain.into_inner(),
                    quality_factor: filter.quality_factor.into_inner(),
                })
                .collect(),
        }
    }
}

impl From<core::ProcessorError> for ProcessorError {
    fn from(error: core::ProcessorError) -> Self {
        match error {
            core::ProcessorError::PreampOutOfRange => Self::PreampOutOfRange,
            core::ProcessorError::FilterAtOrAboveNyquist { .. } => Self::FilterAtOrAboveNyquist,
            core::ProcessorError::UnstableFilter { .. } => Self::UnstableFilter,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use super::{
        Chain, PROCESS_INVALID_BUFFER, PROCESS_INVALID_HANDLE, PROCESS_OK, PROCESS_TOO_MANY_FRAMES,
        Processor, ProcessorError, tunic_processor_process_realtime,
    };

    struct CountingAllocator;

    thread_local! {
        static TRACK_ALLOCATIONS: Cell<bool> = const { Cell::new(false) };
        static ALLOCATION_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    // SAFETY: allocation and deallocation are delegated unchanged to `System`.
    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            TRACK_ALLOCATIONS.with(|tracking| {
                if tracking.get() {
                    ALLOCATION_COUNT.set(ALLOCATION_COUNT.get() + 1);
                }
            });
            // SAFETY: this implementation forwards the allocator contract.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: `pointer` and `layout` came from `System` above.
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    fn count_allocations(operation: impl FnOnce()) -> usize {
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
        ALLOCATION_COUNT.with(|count| count.set(0));
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(true));
        operation();
        TRACK_ALLOCATIONS.with(|tracking| tracking.set(false));
        ALLOCATION_COUNT.get()
    }

    fn flat(preamp_gain_db: f64) -> Chain {
        Chain {
            preamp_gain_db,
            filters: Vec::new(),
        }
    }

    #[test]
    fn realtime_entry_processes_the_callers_buffer_in_place() {
        let processor = Processor::new(48_000.0, 2, flat(6.0), false).unwrap();
        let handle = processor.realtime_handle();
        let mut samples = [0.25, -0.5, 0.125, -0.25];

        // SAFETY: `processor` remains alive and this test owns callback access.
        let status = unsafe {
            tunic_processor_process_realtime(handle, samples.as_mut_ptr(), samples.len() / 2)
        };

        let gain = 10.0_f32.powf(6.0 / 20.0);
        assert_eq!(status, PROCESS_OK);
        assert!((samples[0] - 0.25 * gain).abs() < f32::EPSILON);
        assert!((samples[1] + 0.5 * gain).abs() < f32::EPSILON);
    }

    #[test]
    fn realtime_entry_does_not_allocate() {
        let processor = Processor::new(48_000.0, 2, flat(0.0), false).unwrap();
        let handle = processor.realtime_handle();
        let mut samples = [0.25, -0.5, 0.125, -0.25];

        let allocations = count_allocations(|| {
            // SAFETY: `processor` remains alive and this test owns callback access.
            assert_eq!(
                unsafe {
                    tunic_processor_process_realtime(
                        handle,
                        samples.as_mut_ptr(),
                        samples.len() / 2,
                    )
                },
                PROCESS_OK
            );
        });

        assert_eq!(allocations, 0);
    }

    #[test]
    fn realtime_entry_rejects_invalid_inputs_before_core_processing() {
        let processor = Processor::new(48_000.0, 1, flat(0.0), false).unwrap();
        let handle = processor.realtime_handle();

        assert_eq!(
            // SAFETY: a null-equivalent token is intentionally tested and rejected.
            unsafe { tunic_processor_process_realtime(0, std::ptr::null_mut(), 0) },
            PROCESS_INVALID_HANDLE
        );
        assert_eq!(
            // SAFETY: `processor` remains alive and the null buffer is rejected.
            unsafe { tunic_processor_process_realtime(handle, std::ptr::null_mut(), 1) },
            PROCESS_INVALID_BUFFER
        );
        let mut samples = [0.0; 4];
        assert_eq!(
            // SAFETY: `processor` remains alive and excess frames are rejected.
            unsafe { tunic_processor_process_realtime(handle, samples.as_mut_ptr(), 2) },
            PROCESS_TOO_MANY_FRAMES
        );
    }

    #[test]
    fn control_updates_are_applied_by_the_realtime_entry() {
        let processor = Processor::new(48_000.0, 240, flat(0.0), false).unwrap();
        let controller = processor.controller();
        controller.set_chain(flat(-6.0)).unwrap();
        let handle = processor.realtime_handle();
        let mut transition = [1.0; 240 * 2];

        // SAFETY: `processor` remains alive and this test owns callback access.
        assert_eq!(
            unsafe {
                tunic_processor_process_realtime(
                    handle,
                    transition.as_mut_ptr(),
                    transition.len() / 2,
                )
            },
            PROCESS_OK
        );
        let mut settled = [1.0, -0.5];
        // SAFETY: the same exclusive callback ownership continues.
        assert_eq!(
            unsafe {
                tunic_processor_process_realtime(handle, settled.as_mut_ptr(), settled.len() / 2)
            },
            PROCESS_OK
        );

        let gain = 10.0_f32.powf(-6.0 / 20.0);
        assert!((settled[0] - gain).abs() < f32::EPSILON);
        assert!((settled[1] + 0.5 * gain).abs() < f32::EPSILON);
    }

    #[test]
    fn construction_rejects_invalid_domain_values() {
        assert!(matches!(
            Processor::new(0.0, 1, flat(0.0), false),
            Err(ProcessorError::InvalidSampleRate)
        ));
        assert!(matches!(
            Processor::new(48_000.0, 0, flat(0.0), false),
            Err(ProcessorError::InvalidMaximumFrameCount)
        ));
    }
}
