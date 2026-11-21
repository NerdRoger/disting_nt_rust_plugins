"""Capture KickLab's KLD v1 diagnostics from USB MIDI. Sends no MIDI."""
import argparse
import json
import sys
import time
from pathlib import Path

PREFIX = (0x7D, 0x4B, 0x4C, 0x44, 1)
# Little-endian base-128 integers, with F0 and F7 stripped by Mido.
FIELDS = (
    ("build", 32), ("instance", 14), ("session", 32), ("window", 32),
    ("start_us", 64), ("duration_us", 32), ("gap_count", 32), ("gap_cycles", 64),
    ("min_gap_cycles", 32), ("max_gap_cycles", 32), ("missed_reads", 32),
    ("skipped_voice_frames", 32), ("worker_wait_frames", 32), ("buffer_wait_frames", 32),
    ("peak_mv", 32), ("above_one_hit_blocks", 32), ("add_mode_blocks", 32),
    ("sample_rate", 32), ("block_frames", 14), ("render_calls", 32),
    ("render_cycles", 64),
)


def decode(data):
    if tuple(data[:len(PREFIX)]) != PREFIX or len(data) != 119:
        raise ValueError("wrong KLD header, version, or length")
    offset = len(PREFIX)
    result = {}
    for name, bits in FIELDS:
        digits = data[offset:offset + (bits + 6) // 7]
        value = sum(digit << (7 * index) for index, digit in enumerate(digits))
        if any(not 0 <= digit < 128 for digit in digits) or value >= 1 << bits:
            raise ValueError(f"invalid KLD field {name}")
        result[name] = value
        offset += len(digits)
    if result["duration_us"] == 0 or result["sample_rate"] == 0:
        raise ValueError("invalid sample clock")
    count = result["gap_count"]
    if not result["min_gap_cycles"] * count <= result["gap_cycles"] <= result["max_gap_cycles"] * count:
        raise ValueError("invalid callback gap aggregate")
    return result


def summarize(rows, elapsed):
    if not rows:
        raise ValueError("no matching diagnostic windows")
    first = rows[0]
    keys = ("build", "instance", "session", "sample_rate", "block_frames")
    identity = {key: first[key] for key in keys}
    if any(any(row[key] != value for key, value in identity.items()) for row in rows):
        raise ValueError("multiple streams or inconsistent stream metadata")
    if any(current["start_us"] < previous["start_us"] + previous["duration_us"]
           for previous, current in zip(rows, rows[1:])):
        raise ValueError("duplicate or overlapping diagnostic windows")
    sum_fields = ("duration_us", "gap_count", "gap_cycles", "missed_reads",
                  "skipped_voice_frames", "worker_wait_frames", "buffer_wait_frames",
                  "above_one_hit_blocks", "add_mode_blocks", "render_calls", "render_cycles")
    totals = {field: sum(row[field] for row in rows) for field in sum_fields}
    span_us = rows[-1]["start_us"] + rows[-1]["duration_us"] - first["start_us"]
    return dict(identity=identity, host_seconds=elapsed, windows=len(rows),
                missing_windows=sum((b["window"] - a["window"] - 1) & 0xFFFF_FFFF
                                    for a, b in zip(rows, rows[1:])),
                missing_device_us=span_us - totals["duration_us"], totals=totals,
                largest_gap=max(rows, key=lambda row: row["max_gap_cycles"]),
                highest_peak=max(rows, key=lambda row: row["peak_mv"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=100)
    parser.add_argument("--port")
    parser.add_argument("--build", type=lambda value: int(value, 0))
    parser.add_argument("--instance", type=int, default=1)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    args = parser.parse_args()
    if not 0 < args.seconds < float("inf"):
        parser.error("--seconds must be finite and positive")
    try:
        import mido
        ports = mido.get_input_names()
        matches = [p for p in ports if (p == args.port if args.port else "disting nt" in p.lower())]
        if len(matches) != 1:
            raise ValueError(f"expected one Disting NT MIDI input, got {matches!r}; available: {ports!r}")
        if args.summary.exists():
            raise FileExistsError(args.summary)
        started = time.monotonic()
        first = last = None
        rows = []
        with args.output.open("x", encoding="utf-8") as raw, mido.open_input(matches[0]) as port:
            while first is None or time.monotonic() - first < args.seconds:
                now = time.monotonic()
                if (first is None and now - started >= 10) or (last is not None and now - last >= 6):
                    raise TimeoutError("diagnostic stream timed out")
                for message in port.iter_pending():
                    if message.type != "sysex" or tuple(message.data[:4]) != PREFIX[:4]:
                        continue
                    row = decode(message.data)
                    if row["instance"] != args.instance or (args.build is not None and row["build"] != args.build):
                        continue
                    last = time.monotonic()
                    if first is None:
                        first = last
                        print(f"Receiving KickLab diagnostics on {matches[0]}", file=sys.stderr, flush=True)
                    row["received_seconds"] = last - started
                    rows.append(row)
                    raw.write(json.dumps(dict(decoded=row, data=list(message.data))) + "\n")
                    raw.flush()
                time.sleep(0.005)
        summary = summarize(rows, time.monotonic() - first)
        with args.summary.open("x", encoding="utf-8") as output:
            json.dump(summary, output, indent=2)
            output.write("\n")
        print(json.dumps(summary, indent=2))
        return 0
    except (OSError, ValueError, RuntimeError, ImportError) as exc:
        print(f"Capture failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
