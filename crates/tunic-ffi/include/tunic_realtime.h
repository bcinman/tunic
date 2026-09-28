#ifndef TUNIC_REALTIME_H
#define TUNIC_REALTIME_H

#include <stddef.h>
#include <stdint.h>

#define TUNIC_PROCESS_OK ((uint8_t)0)
#define TUNIC_PROCESS_INVALID_HANDLE ((uint8_t)1)
#define TUNIC_PROCESS_INVALID_BUFFER ((uint8_t)2)
#define TUNIC_PROCESS_TOO_MANY_FRAMES ((uint8_t)3)

uint8_t tunic_processor_process_realtime(
    uint64_t processor_handle,
    float *interleaved_stereo,
    size_t frame_count
);

#endif
