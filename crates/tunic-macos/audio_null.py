"""Generate a quiet stereo probe and compare PCM WAV loopback captures without gain fitting.

Only fixed integer latency is removed. Resampling, fractional delay, clock drift,
channel swaps, and gain changes remain visible rather than being fitted away.
"""
import argparse
import json
import math
import random
import wave
from array import array


def stimulus(path, rate=48000):
    rng = random.Random(7319)
    payload = bytearray()
    for frame in range(rate * 6):
        position = frame - rate // 2
        envelope = max(0, min(1, position / (rate * 0.02), (rate * 5 - position) / (rate * 0.02)))
        for _ in range(2):
            sample = round(rng.uniform(-0.1, 0.1) * envelope * (2**23 - 1))
            payload.extend(sample.to_bytes(3, "little", signed=True))
    with wave.open(str(path), "wb") as output:
        output.setparams((2, 3, rate, 0, "NONE", "not compressed"))
        output.writeframes(payload)


def read_pcm(path):
    with wave.open(str(path), "rb") as source:
        width = source.getsampwidth()
        if source.getnchannels() != 2 or width not in (2, 3, 4):
            raise ValueError("Expected stereo signed 16/24/32-bit PCM WAV (not float WAV)")
        rate = source.getframerate()
        data = source.readframes(source.getnframes())
    channels = [array("d"), array("d")]
    for offset in range(0, len(data), width * 2):
        for channel in range(2):
            start = offset + channel * width
            value = int.from_bytes(data[start:start + width], "little", signed=True)
            channels[channel].append(value / 2 ** (width * 8 - 1))
    return rate, channels


def active_range(signal):
    peak = max(map(abs, signal), default=0)
    if peak < 0.001:
        raise ValueError("Recording is silent or too quiet for a reliable null test")
    threshold = peak * 0.02
    first = next(index for index, value in enumerate(signal) if abs(value) > threshold)
    last = next(index for index in range(len(signal) - 1, -1, -1) if abs(signal[index]) > threshold)
    return first, last


def lag_at(reference, recorded, start, estimate, radius):
    block = reference[start:start + 1024]
    candidates = range(estimate - radius, estimate + radius + 1)
    candidates = [lag for lag in candidates if 0 <= start + lag
                  and start + lag + len(block) <= len(recorded)]
    if not candidates:
        raise ValueError("Recording does not cover the reference signal")
    return max(candidates, key=lambda lag: sum(
        value * recorded[start + lag + index] for index, value in enumerate(block)))


def decibels(value):
    # JSON has no infinity; null means a mathematically zero residual.
    return 20 * math.log10(value) if value > 0 else None


def compare(reference_path, recorded_path):
    rate, reference = read_pcm(reference_path)
    actual_rate, recorded = read_pcm(recorded_path)
    if rate != actual_rate:
        raise ValueError(f"Sample rates differ: {rate} versus {actual_rate}; do not resample to hide it")
    first, last = active_range(reference[0])
    actual_first, _ = active_range(recorded[0])
    if last - first < rate * 2:
        raise ValueError("Need at least two seconds of active reference audio")
    anchor = first + rate // 10
    lag = lag_at(reference[0], recorded[0], anchor, actual_first - first, 256)
    if first + lag < 0 or last + lag >= len(recorded[0]):
        raise ValueError("Recording truncates the reference signal")
    drift = [lag_at(reference[0], recorded[0], point, lag, 64) - lag
             for point in [(first + last) // 2, last - 2048]]
    results = []
    for channel in range(2):
        expected = reference[channel][first:last + 1]
        actual = recorded[channel][first + lag:last + lag + 1]
        energy = sum(value * value for value in expected)
        errors = [value - target for value, target in zip(actual, expected)]
        if energy == 0:
            raise ValueError("Both reference channels must contain signal")
        ratio = math.sqrt(sum(error * error for error in errors) / energy)
        gain = math.sqrt(sum(value * value for value in actual) / energy)
        peak_error = max(map(abs, errors))
        clipping = sum(abs(value) >= 0.9999 for value in actual)
        results.append({
            "channel": "left" if channel == 0 else "right",
            "residual_db_relative": decibels(ratio),
            "gain_db": decibels(gain),
            "peak_error": peak_error,
            "clipped_samples": clipping,
            "passed": ratio <= 1e-4 and peak_error <= 1e-4 and clipping == 0,
        })
    return {
        "sample_rate": rate,
        "latency_frames": lag,
        "latency_ms": lag * 1000 / rate,
        "drift_frames_middle_end": drift,
        "channels": results,
        "passed": all(item["passed"] for item in results) and drift == [0, 0],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("generate")
    generate.add_argument("output")
    generate.add_argument("--rate", type=int, choices=[44100, 48000, 96000, 192000], default=48000)
    null = commands.add_parser("compare")
    null.add_argument("reference")
    null.add_argument("recording")
    args = parser.parse_args()
    try:
        if args.command == "generate":
            stimulus(args.output, args.rate)
        else:
            result = compare(args.reference, args.recording)
            print(json.dumps(result, indent=2, allow_nan=False))
            raise SystemExit(0 if result["passed"] else 1)
    except (ValueError, wave.Error, OSError) as error:
        parser.exit(2, f"{error}\n")


if __name__ == "__main__":
    main()
