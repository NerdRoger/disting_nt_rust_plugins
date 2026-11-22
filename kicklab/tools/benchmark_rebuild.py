"""Drive repeatable KickLab edits over MIDI and capture profiling."""

import argparse
import importlib
import json
import sys
import time
from contextlib import contextmanager
from pathlib import Path

import monitor_diagnostics

HEADER = (0, 0x21, 0x27, 0x6D)
TUNE = "L1: Tail Tune"
DRIVE = "L1: Drive"
CURVE = "L1: Sweep Curve"
CLICK = "L1: Click Level"
LOWPASS_OFF = 101
LONG_DECAY = {
    "L1: Body Decay": 1000, DRIVE: 170, "L1: Sweep Depth": 23,
    "L1: Sweep Time": 23, CURVE: 55, CLICK: 71, "L1: Click Tone": 2214,
    "L1: Click Decay": 109,
}
REFERENCE = {
    "Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 500,
    "L1: Body Decay": 460,
    DRIVE: 20, "L1: Sweep Depth": 24, "L1: Sweep Time": 24, CURVE: 0,
    CLICK: 24, "L1: Click Tone": 5500, "L1: Click Decay": 60,
}
CASES = {
    "tail-tune": ({}, TUNE, (420, 650, 530, 750)),
    "tail-tune-drive-off": ({DRIVE: 0}, TUNE, (420, 650, 530, 750)),
    "tail-tune-curved": ({CURVE: -50}, TUNE, (420, 650, 530, 750)),
    "tail-tune-long": (LONG_DECAY, TUNE, (420, 650, 530, 750)),
    "tail-tune-long-lowpass": (LONG_DECAY | {"Low-pass": 0}, TUNE, (420, 650, 530, 750)),
    "tail-tune-long-boost": (LONG_DECAY | {"Master Gain": 120}, TUNE, (420, 650, 530, 750)),
    "click-level": ({}, CLICK, (12, 32, 22, 42)),
    "click-level-long": (LONG_DECAY, CLICK, (15, 85, 25, 95, 35, 75)),
    "click-level-ramp": (
        {}, CLICK, tuple(range(15, 86, 2)) + tuple(range(83, 15, -2)),
    ),
}
MODAL_REFERENCE = {
    "Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 570,
    "L1: Body Decay": 350, DRIVE: 10, "L1: Upper Modes": 62, "L1: Damping": 44,
    "L1: Strike Position": 50, "L1: Beater Level": 60, "L1: Beater Hardness": 45,
}
MODAL_CASES = {
    "tail-tune": ({}, TUNE, (420, 650, 530, 750)),
    "tail-tune-drive-off": ({DRIVE: 0}, TUNE, (420, 650, 530, 750)),
    "damping": ({}, "L1: Damping", (25, 50, 75, 40)),
    "beater-level": ({}, "L1: Beater Level", (15, 75, 35, 90)),
    "beater-hardness": ({}, "L1: Beater Hardness", (0, 45, 100, 70)),
    "tail-tune-upper-modes-off": ({"L1: Upper Modes": 0}, TUNE, (420, 650, 530, 750)),
}
BASE_TUNE = "L1: Base Tune"
BIT_REFERENCE = {
    "Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, BASE_TUNE: 480,
    "L1: Body Decay": 370, DRIVE: 30, "L1: Sweep Depth": 25, "L1: Sweep Time": 32,
    "L1: Ramp Ratio": 205, "L1: Sample Clock": 3560, "L1: Ramp Bits": 5,
    "L1: Logic Operation": 0, "L1: Tone Cutoff": 1600,
}
BIT_CASES = {
    "base-tune": ({}, BASE_TUNE, (420, 650, 530, 750)),
    "body-decay": ({}, "L1: Body Decay", (250, 500, 350, 750)),
    "base-tune-zero-sweep": ({"L1: Sweep Depth": 0}, BASE_TUNE, (420, 650, 530, 750)),
    "base-tune-high-clock": (
        {"L1: Sample Clock": 15000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
         "L1: Ramp Bits": 12},
        BASE_TUNE, (420, 650, 530, 750),
    ),
    "tone-cutoff": ({}, "L1: Tone Cutoff", (600, 3500, 1200, 6500)),
}
SYNC_DEFAULTS = {
    TUNE: 510, "L1: Body Decay": 410, DRIVE: 50, "L1: Sweep Depth": 21,
    "L1: Sweep Time": 34, "L1: Sync Bend": 55, "L1: Sync Decay": 110,
    "L1: Tail Ratio": 100,
}
SYNC_REFERENCE = {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0} | SYNC_DEFAULTS
SYNC_CASES = {
    "tail-tune": ({}, TUNE, (420, 650, 530, 750)),
    "tail-tune-sync-defaults": (SYNC_DEFAULTS, TUNE, (420, 650, 530, 750)),
    "tail-tune-sync-steady": (
        {"L1: Sweep Depth": 0, "L1: Sync Bend": 0},
        TUNE, (420, 650, 530, 750),
    ),
    "tail-tune-sync-deep": (
        {"L1: Sweep Depth": 60, "L1: Sync Bend": 120, "L1: Sweep Time": 200,
         "L1: Sync Decay": 500, "L1: Tail Ratio": 585, "L1: Body Decay": 1000},
        TUNE, (420, 650, 530, 750),
    ),
}
GRAIN_DEFAULTS = {
    TUNE: 550, "L1: Body Decay": 470, DRIVE: 40, "L1: Sweep Depth": 20,
    "L1: Sweep Time": 70, "L1: Grain Size": 29, "L1: Grain Rate": 107,
    "L1: Density Envelope": 0, "L1: Pitch Spread": 48, "L1: Timing Jitter": 35,
}
GRAIN_REFERENCE = {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0} | GRAIN_DEFAULTS
GRAIN_CASES = {
    "tail-tune": ({}, TUNE, (420, 650, 530, 750)),
    "grain-rate": ({}, "L1: Grain Rate", (70, 150, 105, 210)),
    "drive": ({}, DRIVE, (20, 75, 40, 140)),
    "grain-defaults": (GRAIN_DEFAULTS, TUNE, (420, 650, 530, 750)),
    "grain-dense": (
        {"L1: Body Decay": 1000, "L1: Grain Size": 100, "L1: Grain Rate": 220,
         "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
         "L1: Density Envelope": 30, "L1: Pitch Spread": 240, "L1: Timing Jitter": 100},
        TUNE, (420, 650, 530, 750),
    ),
    "grain-zero-modulation": (
        {"L1: Sweep Depth": 0, "L1: Pitch Spread": 0, "L1: Density Envelope": 0},
        TUNE, (420, 650, 530, 750),
    ),
    "grain-short-drive": (
        {"L1: Body Decay": 120}, DRIVE, (20, 75, 40, 140),
    ),
}
MODELS = {
    "pitch-sweep": ("Pitch Sweep", 1, REFERENCE, CASES),
    "modal-drum": ("Modal Drum", 2, MODAL_REFERENCE, MODAL_CASES),
    "bit-logic": ("Bit Logic", 3, BIT_REFERENCE, BIT_CASES),
    "hard-sync": ("Hard Sync", 5, SYNC_REFERENCE, SYNC_CASES),
    "grain-cloud": ("Grain Cloud", 4, GRAIN_REFERENCE, GRAIN_CASES),
    "resonator": (
        "Resonator", 6,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 530,
         "L1: Body Decay": 820, DRIVE: 10, "L1: Sweep Depth": 13, "L1: Sweep Time": 12,
         "L1: Tail Pitch Drift": 15, "L1: Excitation": 65, "L1: Exciter Width": 50,
         "L1: Tone Cutoff": 2000},
        {
            "resonator-default": ({}, TUNE, (420, 650, 530, 750)),
            "resonator-deep": (
                {"L1: Body Decay": 1000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
                 "L1: Tail Pitch Drift": 60, "L1: Exciter Width": 300},
                TUNE, (420, 650, 530, 750),
            ),
        },
    ),
    "two-operator-fm": (
        "Two-Operator FM", 7,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 490,
         "L1: Body Decay": 420, DRIVE: 30, "L1: Sweep Depth": 24, "L1: Sweep Time": 22,
         "L1: FM Amount": 320, "L1: Modulator Ratio": 345, "L1: FM Decay": 78,
         "L1: Modulator Sweep": 0},
        {
            "fm-default": ({}, TUNE, (420, 650, 530, 750)),
            "fm-deep": (
                {"L1: Body Decay": 1000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
                 "L1: FM Amount": 900, "L1: Modulator Ratio": 1600,
                 "L1: FM Decay": 400, "L1: Modulator Sweep": 36},
                TUNE, (420, 650, 530, 750),
            ),
            "fm-amount-off": ({"L1: FM Amount": 0}, TUNE, (420, 650, 530, 750)),
        },
    ),
    "noise-sweep": (
        "Noise Sweep", 8,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, "L1: Tail Center": 580,
         "L1: Body Decay": 260, DRIVE: 40, "L1: Sweep Depth": 41, "L1: Sweep Time": 27,
         "L1: Filter Q": 41, "L1: Q Sweep": 0, "L1: Grain": 12},
        {
            "noise-default": ({}, "L1: Tail Center", (420, 650, 530, 750)),
            "noise-deep": (
                {"L1: Body Decay": 1000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
                 "L1: Filter Q": 5, "L1: Q Sweep": -30, "L1: Grain": 100},
                "L1: Tail Center", (420, 650, 530, 750),
            ),
            "noise-steady": (
                {"L1: Sweep Depth": 0, "L1: Q Sweep": 0, "L1: Grain": 0},
                "L1: Tail Center", (420, 650, 530, 750),
            ),
        },
    ),
    "ring-mod": (
        "Ring Mod", 9,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 520,
         "L1: Body Decay": 420, DRIVE: 30, "L1: Sweep Depth": 17, "L1: Sweep Time": 32,
         "L1: Lower Osc Ratio": 350, "L1: Clang Level": 65, "L1: Clang Decay": 102,
         "L1: Clang Pitch Env": 0},
        {
            "ring-default": ({}, TUNE, (420, 650, 530, 750)),
            "ring-deep": ({"L1: Body Decay": 1000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
                           "L1: Lower Osc Ratio": 900, "L1: Clang Level": 100,
                           "L1: Clang Decay": 600, "L1: Clang Pitch Env": -24},
                          TUNE, (420, 650, 530, 750)),
        },
    ),
    "wavefold": (
        "Wavefold", 10,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 490,
         "L1: Body Decay": 430, DRIVE: 10, "L1: Sweep Depth": 16, "L1: Sweep Time": 28,
         "L1: Fold Amount": 42, "L1: Fold Decay": 158, "L1: Fold Bias": 26,
         "L1: Clean Sub Blend": 0},
        {
            "fold-default": ({}, TUNE, (420, 650, 530, 750)),
            "fold-deep": ({"L1: Body Decay": 1000, "L1: Sweep Depth": 60, "L1: Sweep Time": 200,
                           "L1: Fold Amount": 100, "L1: Fold Decay": 650, "L1: Fold Bias": -80},
                          TUNE, (420, 650, 530, 750)),
            "fold-clean": ({"L1: Clean Sub Blend": 100}, TUNE, (420, 650, 530, 750)),
        },
    ),
    "delay-drum": (
        "Delay Drum", 11,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 640,
         "L1: Body Decay": 520, DRIVE: 50, "L1: Loop Cutoff": 1700, "L1: Strike Mix": 100,
         "L1: Strike Width": 57, "L1: Strike Tone": 2600},
        {
            "delay-default": ({}, TUNE, (420, 650, 530, 750)),
            "delay-long": ({"L1: Body Decay": 1000, "L1: Loop Cutoff": 250,
                            "L1: Strike Mix": 0, "L1: Strike Width": 200,
                            "L1: Strike Tone": 8500}, TUNE, (250, 325, 460, 640)),
        },
    ),
    "spectral-pulse": (
        "Spectral Pulse", 12,
        {"Trigger": 0, "Low-pass": LOWPASS_OFF, "Master Gain": 0, TUNE: 540,
         "L1: Body Decay": 345, DRIVE: 20, "L1: Snap Level": 75, "L1: Snap Center": 720,
         "L1: Snap Bandwidth": 38, "L1: Dispersion": 12},
        {
            "spectral-default": ({}, TUNE, (420, 650, 530, 750)),
            "spectral-wide": ({"L1: Body Decay": 850, "L1: Snap Level": 100,
                               "L1: Snap Center": 3800, "L1: Snap Bandwidth": 100,
                               "L1: Dispersion": 200}, TUNE, (420, 650, 530, 750)),
            "spectral-bass": ({"L1: Snap Level": 0}, TUNE, (420, 650, 530, 750)),
        },
    ),
}
NTP_SEQUENCE = slice(13, 18)


def encode16(value):
    return (value >> 14 & 3, value >> 7 & 127, value & 127)


def decode16(data):
    high, middle, low = data
    value = (high & 3) << 14 | middle << 7 | low
    return value - 65536 if value >= 32768 else value


def choose_port(names, requested):
    candidates = [name for name in names
                  if (name == requested if requested else "disting nt" in name.lower())]
    if len(candidates) != 1:
        raise ValueError(f"Expected one Disting NT port, found {candidates!r} in {names!r}")
    return candidates[0]


@contextmanager
def polling_input(mido, name):
    # Windows RtMidi can stall when a port closes with a receive callback
    # active. The benchmark only polls, so use RtMidi's queue directly.
    import rtmidi

    midi_in = rtmidi.MidiIn()
    midi_in.ignore_types(sysex=False)
    midi_in.open_port(midi_in.get_ports().index(name))

    class Input:
        @staticmethod
        def poll():
            received = midi_in.get_message()
            return mido.Message.from_bytes(received[0]) if received is not None else None

    try:
        yield Input()
    finally:
        midi_in.close_port()


class Device:
    def __init__(self, mido, rx, tx, capture, raw, args):
        self.mido = mido
        self.rx = rx
        self.tx = tx
        self.capture = capture
        self.raw = raw
        self.header = (*HEADER, args.sysex_id)
        self.algorithm = args.algorithm
        self.instance = args.instance
        self.events = []
        self.diagnostics = []
        self.started = time.monotonic()

    def record(self, **event):
        event["received_seconds"] = time.monotonic() - self.started
        self.raw.write(json.dumps(event) + "\n")
        self.raw.flush()
        return event

    def send(self, command, *data):
        self.tx.send(self.mido.Message("sysex", data=(*self.header, command, *data)))
        self.record(sent=[command, *data])

    def receive(self):
        message = self.rx.poll()
        if message is None:
            time.sleep(0.002)
            return None
        if message.type != "sysex":
            return None
        data = tuple(message.data)
        if data[:len(monitor_diagnostics.PREFIX)] == monitor_diagnostics.PREFIX:
            diagnostics = monitor_diagnostics.decode(data)
            if diagnostics["instance"] == self.instance:
                self.diagnostics.append(diagnostics)
                self.record(data=list(data), diagnostics=diagnostics)
            return data
        row = self.capture.decode(data)
        if row is not None and row["instance"] == self.instance:
            self.events.append(self.record(data=list(data), decoded=row))
        return data

    def wait(self, seconds):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            self.receive()

    def request(self, command, *payload):
        self.send(command, *payload)
        prefix = (*self.header, command, *payload)
        until = time.monotonic() + 3
        while time.monotonic() < until:
            data = self.receive()
            if data is not None and data[:len(prefix)] == prefix:
                return bytes(data[len(prefix):])
        raise TimeoutError(f"No reply to {command:02X} {payload!r}")

    def read(self, index):
        return decode16(self.request(0x45, self.algorithm, *encode16(index)))

    def write(self, index, value):
        self.send(0x46, self.algorithm, *encode16(index), *encode16(value))

    def parameters(self, model="pitch-sweep"):
        if self.request(0x40, self.algorithm)[:4] != b"KLab":
            raise ValueError("Selected algorithm is not KickLab")
        count = decode16(self.request(0x42, self.algorithm))
        values = self.request(0x44, self.algorithm)
        valid_counts = (18, 22, 30, 37, 42, 52, 54, 55, 56, 57, 58, 60, 62, 64, 66, 67,
                        82, 97, 112, 127, 23, 38, 53, 68, 83, 98, 113, 128)
        if count not in valid_counts or len(values) != count * 3:
            raise ValueError("Unexpected KickLab parameter layout")
        params = {}
        for index in range(count):
            info = self.request(0x43, self.algorithm, *encode16(index))
            name = info[10:].split(b"\0", 1)[0].decode("ascii")
            params[name] = (index, decode16(info[:3]), decode16(info[3:6]),
                            decode16(values[3 * index:3 * index + 3]))
        model_name, model_value, _, _ = MODELS[model]
        if (params["Bypass"][3] or params["L1: Model"][3] != model_value or params["L1: Level"][3] == 0
                or params.get("L1: Mute", (0, 0, 0, 0))[3]
                or any(spec[3] for name, spec in params.items()
                       if name.endswith(": Model") and name != "L1: Model")):
            raise ValueError(f"Benchmark requires one audible {model_name} in layer 1 and bypass off")
        return params


def measure(device, name, mode, parameter, values, args, reference, params, touched, metadata):
    def set_value(key, value):
        index, minimum, maximum, original = params[key]
        if not minimum <= value <= maximum:
            raise ValueError(f"{key} value {value} outside device range")
        if device.read(index) != value:
            touched[index] = original
            device.write(index, value)
            if device.read(index) != value:
                raise ValueError(f"{key} did not accept {value}")

    if args.preserve_setup:
        settings = {key: params[key][3] for key in reference}
        if args.model != "pitch-sweep":
            settings.update(mode)
    else:
        settings = reference | mode
    if args.playback:
        settings["Trigger"] = params["Trigger"][3]
        if settings["Trigger"] == 0:
            raise ValueError("Playback benchmark requires an assigned trigger bus")
    else:
        settings["Trigger"] = 0
    for key, value in settings.items():
        set_value(key, value)
    device.wait(3)
    first = len(device.events)
    writes = 0
    start = time.monotonic()
    index, minimum, maximum, original = params[parameter]
    if not all(minimum <= value <= maximum for value in values):
        raise ValueError(f"{parameter} test values outside device range")
    touched[index] = original
    while time.monotonic() - start < args.seconds:
        value = values[writes % len(values)]
        device.write(index, value)
        if device.read(index) != value:
            raise ValueError(f"{parameter} did not accept {value}")
        writes += 1
        device.wait(max(0, start + writes * args.interval - time.monotonic()))
    workload = dict(case=name, values=settings, swept_parameter=parameter,
                    sequence=values, writes=writes, write_interval_seconds=args.interval)
    if args.midi_only:
        print(f"{name}: {writes} verified MIDI edits", flush=True)
        return workload
    last = len(device.events)
    events = device.events[first:last]
    # USB reports arrive after a window closes. Use complete interior windows,
    # excluding the first two for warm-up and the final boundary window.
    windows = sorted({event["decoded"]["window"] for event in events
                      if event["decoded"]["kind"] == "metric"})
    if len(windows) < 4:
        raise ValueError(f"Too few metric windows for {name}")
    keep = set(windows[2:-1])
    picked = [event for event in events if event["decoded"].get("window") in keep]
    first_seq = picked[0]["decoded"]["sequence"]
    last_seq = picked[-1]["decoded"]["sequence"]
    # Preserve interleaved metadata and include a copy of the registry.
    measured = [event for event in events
                if first_seq <= event["decoded"]["sequence"] <= last_seq]
    registry = [{"data": event["data"]} for event in metadata.values()]
    for sequence, event in enumerate(registry, start=first_seq - len(registry)):
        event["data"] = event["data"].copy()
        event["data"][NTP_SEQUENCE] = [(sequence & 0xffffffff) >> (7 * i) & 127
                                       for i in range(5)]
    summary = device.capture.summarize(registry + measured)
    if summary["status"] != "ok" or len(summary["streams"]) != 1:
        raise ValueError(f"Unusable telemetry for {name}: {summary}")
    stream = summary["streams"][0]
    if any(function["windows"] != len(keep) for function in stream["functions"]):
        raise ValueError(f"Incomplete metric windows for {name}")
    render = next(function for function in stream["functions"] if function["function_id"] == 2)
    if bool(render["calls"]) == args.expect_no_rebuilds:
        raise ValueError(f"Unexpected number of rebuilds for {name}: {render['calls']}")
    lowpass = next(function for function in stream["functions"] if function["function_id"] == 5)
    expected_lowpass = render["calls"] if settings["Low-pass"] != LOWPASS_OFF else 0
    if lowpass["calls"] != expected_lowpass:
        raise ValueError(f"Unexpected number of Low-pass calls for {name}: {lowpass['calls']}")
    device.wait(0.75)  # KLD arrives after the corresponding NTP window.
    diagnostics = [row for row in device.diagnostics
                   if row["window"] in keep and row["session"] == stream["session"]]
    if {row["window"] for row in diagnostics} != keep:
        raise ValueError(f"Incomplete playback diagnostics for {name}")
    playback_stats = monitor_diagnostics.summarize(diagnostics, args.seconds)
    if args.playback and not playback_stats["highest_peak"]["peak_mv"]:
        raise ValueError(f"No audio output during playback benchmark {name}")
    print(f"{name}: {render['calls']} rebuilds, "
          f"{render['mean_cycles_per_call'] or 0:,.0f} mean cycles, "
          f"{playback_stats['totals']['buffer_wait_frames']:,} buffer wait frames", flush=True)
    return dict(**workload, diagnostics=playback_stats, **stream)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receiver", type=Path, required=True,
                        help="Path to the profiling skill's capture_profile.py")
    parser.add_argument("--output", type=Path, required=True, help="New raw JSONL path")
    parser.add_argument("--summary", type=Path, required=True, help="New summary JSON path")
    parser.add_argument("--input-port")
    parser.add_argument("--output-port")
    parser.add_argument("--sysex-id", type=int, choices=range(127), default=0, metavar="0..126")
    parser.add_argument("--algorithm", type=int, choices=range(128), default=0, metavar="0..127")
    parser.add_argument("--instance", type=int, default=1)
    parser.add_argument("--build", type=lambda value: int(value, 0))
    parser.add_argument("--playback", action="store_true",
                        help="Keep the existing trigger bus enabled")
    parser.add_argument("--interval", type=float, default=0.25,
                        help="Seconds between edits (default: 0.25)")
    parser.add_argument("--preserve-setup", action="store_true",
                        help="Keep current settings except the swept control and any model case override")
    parser.add_argument("--expect-no-rebuilds", action="store_true",
                        help="Verify that an edit-only diagnostic build did not rebuild")
    parser.add_argument("--midi-only", action="store_true",
                        help="Send verified edits to a build without profiling telemetry")
    parser.add_argument("--model", choices=MODELS, default="pitch-sweep")
    workload = parser.add_mutually_exclusive_group()
    workload.add_argument("--case", action="append",
                          choices=sorted({name for _, _, _, cases in MODELS.values() for name in cases}))
    workload.add_argument("--sweep", nargs="+", metavar="FIELD_OR_VALUE",
                          help="A layer 1 field name followed by raw SysEx values")
    parser.add_argument("--seconds", type=int, default=12, choices=range(8, 101),
                        metavar="8..100")
    args = parser.parse_args()
    if not 0.02 <= args.interval <= 10:
        parser.error("--interval must be between 0.02 and 10 seconds")
    if args.midi_only and (args.build is not None or args.expect_no_rebuilds):
        parser.error("--midi-only cannot verify a profiling build or its rebuild count")
    _, _, reference, available_cases = MODELS[args.model]
    if args.case and any(name not in available_cases for name in args.case):
        parser.error(f"{args.model} cases: {', '.join(available_cases)}")
    cases = [(name, available_cases[name]) for name in args.case or available_cases]
    if args.sweep:
        parameter, *raw_values = args.sweep
        if parameter not in reference or not parameter.startswith("L1: "):
            parser.error("--sweep requires a named layer 1 control for the selected model")
        try:
            values = tuple(map(int, raw_values))
        except ValueError:
            parser.error("--sweep values must be integers")
        if len(set(values)) < 2 or any(a == b for a, b in zip(values, values[1:] + values[:1])):
            parser.error("--sweep requires at least two changing values, including on wrap")
        cases = [(parameter, ({}, parameter, values))]

    import mido

    sys.path.insert(0, str(args.receiver.resolve().parent))
    capture = importlib.import_module(args.receiver.stem)
    input_port = choose_port(mido.get_input_names(), args.input_port)
    output_port = choose_port(mido.get_output_names(), args.output_port)
    result = dict(input_port=input_port, output_port=output_port, algorithm=args.algorithm,
                  instance=args.instance, model=args.model, midi_only=args.midi_only,
                  cases=[], restored=False)
    with args.summary.open("x", encoding="utf-8") as summary:
        try:
            with (args.output.open("x", encoding="utf-8") as raw,
                  polling_input(mido, input_port) as rx, mido.open_output(output_port) as tx):
                device = Device(mido, rx, tx, capture, raw, args)
                params = device.parameters(args.model)
                result["original"] = {name: params[name][3] for name in reference}
                result["original"]["L1: Level"] = params["L1: Level"][3]
                device.record(original=result["original"])
                metadata = {}
                if not args.midi_only:
                    device.wait(5)
                    for event in device.events:
                        row = event["decoded"]
                        if row["kind"] != "metric":
                            metadata[(row["kind"], row.get("function_id"))] = event
                    hello = metadata.get(("hello", None), {}).get("decoded")
                    if hello is None or len(metadata) != hello["function_count"] + 1:
                        raise ValueError("Could not identify a running profile build and registry")
                    if args.build is not None and args.build != hello["build"]:
                        raise ValueError(f"Expected build {args.build:#x}, found {hello['build']:#x}")
                    if {event["decoded"]["session"] for event in device.events} != {hello["session"]}:
                        raise ValueError("Multiple profiling sessions on selected instance")
                    print(f"Profiling KickLab build {hello['build']:#010x}", flush=True)
                    if args.playback:
                        print("Waiting for the trigger pattern", flush=True)
                        deadline = time.monotonic() + 180
                        while not any(row["peak_mv"] for row in device.diagnostics[-2:]):
                            if time.monotonic() >= deadline:
                                raise TimeoutError("No output while waiting for the trigger pattern")
                            device.receive()
                touched = {}
                try:
                    for case, settings in cases:
                        result["cases"].append(
                            measure(device, case, *settings, args, reference, params, touched, metadata))
                finally:
                    for index, value in reversed(touched.items()):
                        device.write(index, value)
                    mismatches = [index for index, value in touched.items()
                                  if device.read(index) != value]
                    if mismatches:
                        raise ValueError(f"Restoration readback failed for parameters {mismatches}")
                    result["restored"] = True
        finally:
            json.dump(result, summary, indent=2)
            summary.write("\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError) as error:
        sys.exit(str(error))
