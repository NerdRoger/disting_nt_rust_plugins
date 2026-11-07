#!/usr/bin/env python3
"""Collect opt-in Wavesmith USB cycle telemetry; never play notes or edit parameters.

Requires mido and python-rtmidi. MIDI ports are closed on return, including errors.
The default output lives under the ignored repository target directory.
"""
import argparse
import csv
import json
import math
from pathlib import Path
import statistics
import sys
import time

IDENTIFIER = (0x7D, ord("W"), ord("S"), 1)
FIELDS = ("instance", "sequence", "display_cycles", "total_cycles", "callbacks",
          "frames", "sample_rate", "block_frames", "voices", "midi", "stereo")
WIDTHS = (5, 5, 5, 10, 5, 5, 5, 5, 1, 1, 1)
STAGE_FIELDS = ("callback_cycles", "setup_cycles", "build_cycles", "level_cycles", "output_cycles")
DRAW_FIELDS = ("draw_cycles_total", "draw_calls_total")
UI_FIELDS = tuple(f"{kind}_{metric}" for kind in ("ui_idle", "ui_edit", "ui_setup")
                  for metric in ("cycles_total", "calls_total", "max_cycles"))
LABEL_FIELDS = ("parameter_string_calls_total", "prefix_calls_total")
CSV_FIELDS = FIELDS + STAGE_FIELDS + DRAW_FIELDS + UI_FIELDS + LABEL_FIELDS
WORKLOAD = ("instance", "sample_rate", "block_frames", "voices", "midi", "stereo")


def decode(data):
    """mido SysEx data excludes F0 and F7; ignore unrelated vendor messages."""
    data = tuple(data)
    if data[:4] != IDENTIFIER or len(data) < 5 or data[4] not in (1, 2, 3, 4, 5):
        return None
    if len(data) != ({1: 53, 2: 103, 3: 113, 4: 158, 5: 168}[data[4]]) or any(not 0 <= byte < 128 for byte in data):
        raise ValueError("Malformed Wavesmith telemetry")
    result, offset = {}, 5
    for field, width in zip(FIELDS, WIDTHS):
        result[field] = sum(byte << (7 * i) for i, byte in enumerate(data[offset:offset + width]))
        offset += width
    if result["total_cycles"] > 0xFFFFFFFFFFFFFFFF or any(
        result[key] > 0xFFFFFFFF for key in FIELDS if key != "total_cycles"
    ):
        raise ValueError("Telemetry integer overflow")
    if (not result["callbacks"] or not result["frames"] or not result["sample_rate"]
            or not result["block_frames"] or result["voices"] > 24
            or result["midi"] > 1 or result["stereo"] > 1):
        raise ValueError("Invalid profiling window or workload")
    if data[4] >= 2:
        for field in STAGE_FIELDS:
            result[field] = sum(byte << (7*i) for i, byte in enumerate(data[offset:offset+10]))
            offset += 10
    if any(result.get(field, 0) > 0xFFFFFFFFFFFFFFFF for field in STAGE_FIELDS):
        raise ValueError("Stage counter overflow")
    if data[4] >= 3:
        for field in DRAW_FIELDS:
            result[field] = sum(byte << (7*i) for i, byte in enumerate(data[offset:offset+5]))
            offset += 5
    if data[4] >= 4:
        for field in UI_FIELDS:
            result[field] = sum(byte << (7*i) for i, byte in enumerate(data[offset:offset+5]))
            offset += 5
    if data[4] == 5:
        for field in LABEL_FIELDS:
            result[field] = sum(byte << (7*i) for i, byte in enumerate(data[offset:offset+5]))
            offset += 5
    result["cycles_per_callback"] = result["total_cycles"] / result["callbacks"]
    return result


def summarize(rows, label):
    if not rows:
        raise ValueError("No Wavesmith telemetry received")
    if any(("callback_cycles" in row) != ("callback_cycles" in rows[0]) for row in rows):
        raise ValueError("Mixed telemetry versions")
    workload = {key: rows[0][key] for key in WORKLOAD}
    if any(any(row[key] != value for key, value in workload.items()) for row in rows):
        raise ValueError("Workload changed during measurement")
    values = [row["cycles_per_callback"] for row in rows]
    missed = 0
    for previous, current in zip(rows, rows[1:]):
        delta = (current["sequence"] - previous["sequence"]) & 0xFFFFFFFF
        if delta == 0 or delta > 1000:
            raise ValueError("Duplicate/restarted telemetry stream")
        missed += delta - 1
    summary = dict(label=label, reports=len(rows), missed_reports=missed, workload=workload,
                mean_cycles_per_callback=sum(row["total_cycles"] for row in rows) / sum(row["callbacks"] for row in rows),
                median_cycles_per_callback=statistics.median(values),
                min_cycles_per_callback=min(values), max_cycles_per_callback=max(values),
                stdev_cycles_per_callback=statistics.stdev(values) if len(values) > 1 else 0.0,
                standard_error=statistics.stdev(values) / math.sqrt(len(values)) if len(values) > 1 else 0.0)

    for field in STAGE_FIELDS:
        if field in rows[0]:
            if any(field not in row for row in rows):
                raise ValueError("Mixed telemetry versions")
            values = [row[field]/row["callbacks"] for row in rows]
            summary["mean_"+field] = sum(row[field] for row in rows)/sum(row["callbacks"] for row in rows)
            summary["stderr_"+field] = statistics.stdev(values)/math.sqrt(len(values)) if len(values)>1 else 0.0
    if "draw_calls_total" in rows[0] and len(rows) > 1:
        cycles = sum((b["draw_cycles_total"]-a["draw_cycles_total"]) & 0xffffffff for a,b in zip(rows,rows[1:]))
        calls = sum((b["draw_calls_total"]-a["draw_calls_total"]) & 0xffffffff for a,b in zip(rows,rows[1:]))
        seconds = sum(row["frames"]/row["sample_rate"] for row in rows[1:])
        summary.update(draw_calls=calls, mean_draw_cycles=cycles/calls if calls else None,
                       draw_calls_per_second=calls/seconds, draw_cycles_per_second=cycles/seconds)
    if "ui_idle_calls_total" in rows[0] and len(rows) > 1:
        for kind in ("ui_idle", "ui_edit", "ui_setup"):
            cycles = sum((b[kind+"_cycles_total"]-a[kind+"_cycles_total"]) & 0xffffffff for a,b in zip(rows,rows[1:]))
            calls = sum((b[kind+"_calls_total"]-a[kind+"_calls_total"]) & 0xffffffff for a,b in zip(rows,rows[1:]))
            summary[kind+"_calls"] = calls
            summary[kind+"_mean_cycles"] = cycles/calls if calls else None
            summary[kind+"_max_cycles_since_instantiation"] = max(row[kind+"_max_cycles"] for row in rows)
    if "parameter_string_calls_total" in rows[0] and len(rows) > 1:
        seconds = sum(row["frames"]/row["sample_rate"] for row in rows[1:])
        for field in LABEL_FIELDS:
            calls = sum((b[field]-a[field]) & 0xffffffff for a,b in zip(rows,rows[1:]))
            name = field.removesuffix("_total")
            summary[name] = calls
            summary[name+"_per_second"] = calls/seconds
            summary[name+"_at_first_report"] = rows[0][field]
    return summary


def choose_port(names, requested):
    if requested:
        if requested not in names:
            raise ValueError(f"MIDI port not found: {requested!r}; available: {names}")
        return requested
    matches = [name for name in names if "disting nt" in name.lower()]
    if len(matches) != 1:
        raise ValueError(f"Expected one disting NT port; choose explicitly from {names}")
    return matches[0]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seconds", type=float, default=100)
    parser.add_argument("--warmup", type=float, default=5)
    parser.add_argument("--label", default="baseline")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--input")
    parser.add_argument("--output-port")
    parser.add_argument("--instance", type=int)
    parser.add_argument("--expect-voices", type=int)
    parser.add_argument("--expect-mode", choices=["cv", "midi"])
    parser.add_argument("--expect-stereo", action="store_true")
    parser.add_argument("--compare", type=Path)
    parser.add_argument("--list-ports", action="store_true")
    parser.add_argument("--stop", action="store_true", help="Disable diagnostic streaming and close the output port")
    args = parser.parse_args(argv)
    if args.seconds <= 0 or args.warmup < 0:
        parser.error("seconds must be positive and warmup nonnegative")
    import mido
    if args.list_ports:
        print(json.dumps(dict(inputs=mido.get_input_names(), outputs=mido.get_output_names()), indent=2))
        return 0
    output_name = choose_port(mido.get_output_names(), args.output_port)
    command = lambda value: mido.Message("sysex", data=IDENTIFIER + (value,))
    if args.stop:
        with mido.open_output(output_name) as output:
            output.send(command(0x11))
            time.sleep(0.1)
        return 0
    input_name = choose_port(mido.get_input_names(), args.input)
    path = args.output or Path(__file__).resolve().parents[2] / "target" / "wavesmith-profiling" / (time.strftime("%Y%m%d-%H%M%S") + ".csv")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() or path.with_suffix(".json").exists():
        raise ValueError(f"Refusing to overwrite existing results: {path}")
    rows, workload = [], None
    print(f"Listening on {input_name}; warmup {args.warmup:g}s, measurement {args.seconds:g}s", flush=True)
    with mido.open_output(output_name) as output, mido.open_input(input_name) as input_port, path.open("w", newline="", encoding="utf-8") as file:
        writer = csv.DictWriter(file, fieldnames=["elapsed_seconds"] + list(CSV_FIELDS) + ["cycles_per_callback"])
        writer.writeheader()
        time.sleep(0.1)
        output.send(command(0x10))
        start = last_report = last_progress = time.monotonic()
        try:
            while True:
                now = time.monotonic()
                elapsed = now - start
                for message in input_port.iter_pending():
                    if message.type != "sysex":
                        continue
                    row = decode(message.data)
                    if row is None or (args.instance is not None and row["instance"] != args.instance):
                        continue
                    current = {key: row[key] for key in WORKLOAD}
                    if workload is None:
                        workload = current
                        if args.expect_voices is not None and row["voices"] != args.expect_voices:
                            raise ValueError(f"Expected {args.expect_voices} sounding voices, received {row['voices']}")
                        if args.expect_mode and bool(row["midi"]) != (args.expect_mode == "midi"):
                            raise ValueError(f"Unexpected MIDI/CV mode: {current}")
                        if args.expect_stereo and not row["stereo"]:
                            raise ValueError("Expected stereo routing")
                        print(f"Workload: {workload}", flush=True)
                    elif current != workload:
                        raise ValueError("Workload changed or multiple Wavesmith instances are streaming; use --instance")
                    last_report = now
                    # Discard the first partial second after warmup as well.
                    if elapsed >= args.warmup + 1:
                        row["elapsed_seconds"] = elapsed - args.warmup
                        writer.writerow(row)
                        file.flush()
                        rows.append(row)
                if now - last_report > 6:
                    raise RuntimeError("No telemetry for six seconds: stream unavailable, device disconnected, or plugin not loaded")
                if elapsed >= args.warmup + args.seconds + 1:
                    break
                if now - last_progress >= 10:
                    print(f"{max(0,elapsed-args.warmup):.0f}s: {len(rows)} reports", flush=True)
                    last_progress = now
                time.sleep(0.01)
        finally:
            output.send(command(0x11))
            time.sleep(0.1)
    if len(rows) < max(1, int(args.seconds * 0.9)):
        raise RuntimeError(f"Incomplete measurement: {len(rows)} reports; raw data retained at {path}")
    summary = summarize(rows, args.label)
    summary["requested_seconds"] = args.seconds
    if args.compare:
        baseline = json.loads(args.compare.read_text(encoding="utf-8"))
        if baseline["workload"] != summary["workload"]:
            raise ValueError("Cannot compare different workloads")
        summary["baseline"] = str(args.compare)
        summary["change_percent"] = 100 * (summary["mean_cycles_per_callback"] / baseline["mean_cycles_per_callback"] - 1)
        if "mean_callback_cycles" in summary and "mean_callback_cycles" in baseline:
            summary["callback_change_percent"] = 100 * (summary["mean_callback_cycles"]/baseline["mean_callback_cycles"]-1)
        if summary.get("mean_draw_cycles") and baseline.get("mean_draw_cycles"):
            summary["draw_change_percent"] = 100 * (summary["mean_draw_cycles"]/baseline["mean_draw_cycles"]-1)
    path.with_suffix(".json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2), flush=True)
    print(f"Saved {path} and {path.with_suffix('.json')}", flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, RuntimeError, OSError) as error:
        print(f"Error: {error}", file=sys.stderr)
        raise SystemExit(1)
